# thaw-bridge 設計ドキュメント

- ステータス: 設計 + 最初の垂直スライス実装
- 前提: [thaw-hir](../../crates/thaw-hir), [thaw-llvm/hir_codegen](../../crates/thaw-llvm/src/hir_codegen.rs) の現状（Phase 0〜2）を前提にする
- 位置づけ: プロジェクト概要（トップレベル設計ドキュメント）が「Thaw の本質的価値」と位置づける部分。コンパイラ本体（Phase 0〜2）より多くの時間を投資する対象。

## 1. なぜここが本丸か（おさらい）

Perry は npm を Rust で手動再実装、scriptc は手動 FFI 宣言。Thaw は
「`.d.ts` から C FFI を自動生成し、レジストリで結果を共有する」ことで
差別化する。この設計ドキュメントは、その自動生成エンジンの最初の
形を定める。

## 2. 全体アーキテクチャ：2段階の呼び出し経路

トップレベル設計ドキュメント3.1節の通り、npm パッケージ関数の呼び出しは
2つの経路に分岐する：

```
.d.ts のシグネチャ
       │
       ▼
  型 → C ABI マッピング（本ドキュメント4章）
       │
   ┌───┴───┐
   │ 判定  │
   └───┬───┘
       │
  ┌────┴─────────────────┐
  │                       │
Fast path              Fallback path
（型が全部 Thaw の      （generics / union /
 ネイティブ型に収まる）   コールバック引数 / 複雑な
  │                      オブジェクト型など）
  ▼                       ▼
実ネイティブ関数を        QuickJS-NG 上でパッケージ本体を
直接 FFI 呼び出し         実行し、Bridge が値を marshal
（オーバーヘッドゼロ）      してネイティブ側とやり取り
```

**この設計ドキュメントで実装するのは Fast path のコア機構のみ**：
`.d.ts` パース → 型分類 → `FfiSignature` 生成 → 実際に LLVM が
`extern "C"` 宣言を出して呼び出す、というエンドツーエンドの経路。
QuickJS-NG 統合（Fallback path の実行）は7章でインターフェースだけ
設計し、実装は次のフェーズに送る。

## 3. `.d.ts` パース：tsc API ではなく SWC を使う

トップレベル設計ドキュメントの技術スタック節は「型推論: TypeScript
Compiler API (tsc)」としているが、Bridge の `.d.ts` パースについては
**SWC（thaw-parser を再利用）で行う**ことにする。理由：

- tsc は Node.js 上で動く JavaScript 製ツールであり、Rust バイナリの
  中から使うには Node.js プロセスをサブプロセスとして起動するか、
  何らかのFFI/RPC層が要る。Thaw 全体を「Rust ネイティブで完結させる」
  という一貫した方針（[thaw-runtime](../../crates/thaw-runtime/src/lib.rs)
  が hyper/tokio を避けたのと同じ理由）に反する。
- `.d.ts` の型注釈は構文的にはすでに具体的（`interface`/`type`/
  関数シグネチャの型注釈がその場に書いてある）なので、多くのケースは
  tsc の持つ「他ファイルからの型推論・解決」がなくても SWC の構文木
  だけで十分マッピングできる。

**代償（明示的に認める制限）**: tsc は型を「解決」する（`type Foo =
Bar<Baz>` のようなエイリアスや、複数ファイルにまたがる型定義を
最終的に1つの具体型に潰す）。SWC は構文木を返すだけで型解決はしない。
つまり Thaw の `.d.ts` パーサーは **その場に書いてある構文だけ**を
見て判断する、純粋に構文主導のマッピングになる。型エイリアスや
複数ファイルにまたがる複雑な型は Fallback 判定になりやすい
（誤って「対応不可」と判定するケースが tsc ベースより多くなる）。
これは「速く倒れて QuickJS-NG に委ねる」方向の誤り方なので、実行時
の正しさは損なわれない（誤検知は最適化の機会損失であって、バグでは
ない）。

### 3.1 `declare namespace` の中の関数も抽出する

[docs/design/registry.md](registry.md) の「npm と同じ感覚で使える」
検証の一環で見つかった穴: `parse_dts` は当初トップレベルの
`declare function`/`export declare function` しか見ていなかったが、
実際の npm パッケージ（`qs`）は関数を含む型宣言のほぼ全てを
`declare namespace QueryString { ... }` の中に書いていた。

```ts
export = QueryString;
declare namespace QueryString {
    function stringify(obj: any, options?: IStringifyOptions): string;
    function parse(str: string, options?: IParseOptions): ParsedQs;
}
```

これをそのまま `parse_dts` に通すと関数が0個になる。対策として
`extract_fn_decls`（旧 `extract_fn_decl`、複数形に変更）が
`Decl::TsModule`（namespace 宣言）を見つけたら、その中の
`ModuleItem` を再帰的に辿って関数を集めるようにした -- namespace が
入れ子（`namespace Outer { namespace Inner { function f() {} } }`）
でも対応する。

抽出した関数名は**そのまま**（`QueryString.parse` のような
namespace 修飾はしない）。理由: `export = QueryString;` を持つ
パッケージの実際の JS 実装は、6章の CommonJS ラップ機構が既に
サポートしている「`module.exports = { parse, stringify, ... }` という
オブジェクトの各プロパティをグローバルにフックする」パターンと
一致する（`qs` 自身の `lib/index.js` がまさにこの形）。つまり
namespace 抽出とオブジェクトエクスポートのフックは、お互いを意識せず
そのまま噛み合う -- `.d.ts` 側で名前空間が使われていても、実行時に
グローバルへ hoist される名前は変わらないため。

`interface`/`type` が同じ namespace の中で宣言されているケース
（`qs` はまさにこれ）は今回もスコープ外のまま: `resolve_interfaces`
はトップレベルの `interface` しか見ないため、namespace 内で
定義された補助的な型（オプションオブジェクトの型など）への参照は
「対応不可」判定になり Fallback に倒れる。これは安全な劣化
（4.2節と同じ「速く倒れる」方向）なので、実行時の正しさは失われない。

## 4. 型 → C ABI マッピングエンジン

### 4.1 分類アルゴリズム

`.d.ts` の関数シグネチャ1つに対し、パラメータと戻り値それぞれの
`TsType` を再帰的に `thaw_hir::HirType` へマッピングを試みる
（`thaw-hir::lower::lower_ts_type` と基本的に同じ変換規則を
`thaw-bridge` 側でも使う -- 実装は共有できる）。

**マッピング可能（Fast path 対象）**な `HirType`:

| TS 型 | HirType | 備考 |
|---|---|---|
| `number` | `F64` | |
| `string` | `Str` | |
| `boolean` | `Bool` | |
| `number[]` / `Array<number>` | `Array(F64)` | 要素は number のみ（[[project_thaw_overview]] の既存制限を踏襲） |
| `{ x: number; y: number }` | `Object([("x", F64), ("y", F64)])` | フィールドは number のみ |
| `void` | `Void` | 戻り値のみ |

**マッピング不能（Fallback 判定）**:

- generics（`Array<T>` の `T` が具体化されていない、独自ジェネリック関数）
- union / intersection 型（`string | number` など）
- コールバック引数（`(cb: (err: Error) => void) => void` -- 関数型の
  マーシャリングは別の設計課題で、Bridge の最初のスコープには含めない）
- `string[]` / オブジェクトの非 number フィールド（Phase 1/2 の
  既存制限をそのまま継承。将来 Thaw 本体がこれらをサポートしたら
  Bridge 側の分類も自動的に広がる）
- `interface` 宣言経由の型（`TsInterfaceDecl` は現状 thaw-hir 未対応 --
  `.d.ts` では `interface` が頻出するため、これは Fast path の実用上の
  カバレッジを大きく制限する。次の実装ステップの筆頭候補）
- `any` / `unknown` -- 逆に言えばこれは Thaw の `HirType::Json` に
  マッピングしても良さそうに見えるが、V1 では意図的に対象外とする:
  `Json` は「動的な値を読む」ための型であり、「ネイティブ関数への
  受け渡し」の ABI としては何も保証しないため、C ABI マッピングの
  文脈では「対応不可」の方が正直

関数全体は、**すべての**パラメータと戻り値が Fast path 対象の場合のみ
Fast path 判定になる。1つでも Fallback 対象があれば関数全体が
Fallback。

### 4.2 なぜ「全部揃わないとダメ」なのか

部分的に native / 部分的に QuickJS という呼び出し規約は、marshal
コードの複雑さが一気に跳ね上がる（呼び出し途中で ABI 境界をまたぐ
たびに変換が要る）。V1 は「全部ネイティブ」か「全部 QuickJS」の
二択にすることで、生成コードを単純に保つ。

### 4.3 オーバーロードは名前単位で判定する

実際の npm パッケージ（`ms` など）を registry 経由で試して見つかった
ケース: 同じ関数名に対して `.d.ts` が複数のシグネチャ（オーバーロード）
を持ち、それぞれが**別々に**分類すると異なる判定になることがある
（例: `ms(value: number, options?): string` は Fast path、
`ms(value: string): number` は Fallback）。

各シグネチャを独立に `classify` してそのまま `generate_shim` に渡すと、
同じ名前に対して `declare function ms(...)`（ambient FFI 宣言）と
`function ms(argsArray: Json): Json { ... }`（Fallback wrapper）という
**矛盾する2つのトップレベル宣言**が生成されてしまう。thaw-hir は
同名関数の重複を検出しないため、これはコンパイルエラーにも
リンクエラーにもならず、後に書かれた方が黙って勝つ -- 生成順序
（`.d.ts` 内の記述順）にのみ依存する、誰も意図していない挙動になる。

対策として `classify_all(functions)` を追加した: 名前ごとにグループ化し、
**そのグループにシグネチャが1つしかない場合のみ** `classify` の結果を
そのまま使う。2つ以上あれば（個々の判定結果に関わらず）**必ず
Fallback**に倒す -- Fast path は1つの固定シグネチャを1つのネイティブ
シンボルに対応させる仕組みなので、複数の呼び出し形を持つ名前を表現
できない。一方 Fallback (`callDynamic`) は引数の形を問わず JSON を
そのまま JS 側に渡すだけなので、JS 側の関数が自分でオーバーロードを
処理する限り問題なく動く（実際 `ms` はこれで両方の呼び出し形が動く
ことを確認済み）。`generate_shim`/thaw-cli の Fallback 名前収集は
どちらもこの `classify_all` を使うよう統一した。

### 4.4 型分類と「実際にリンクできるか」は別の問題

4.3節の直後に、より根本的な問題を見つけた: `thaw registry add`
（[registry.md](registry.md)）で取り込む実際の npm パッケージは
**純粋な JS で、対応するネイティブライブラリは存在しない**。にも
関わらず、date-fns の `daysToWeeks(days: number): number` のように
オーバーロードのない単純な primitive のみのシグネチャは、`classify`
だけを見れば普通に Fast path に分類される。これをそのまま
`generate_shim` に渡すと `declare function daysToWeeks(...)` という
ambient FFI 宣言が生成され、実際に呼び出すと

```
undefined reference to `daysToWeeks'
```

というリンクエラーで落ちる -- すぐ隣の `bundle.js` に動く JS 実装が
あるにも関わらず、である。「型シグネチャが Fast path 互換かどうか」と
「実際にリンクできるネイティブシンボルが存在するかどうか」は独立した
問題なのに、`classify`/`classify_all` は前者しか見ていなかった。

対策として `generate_shim`/`effective_classifications` に
`native_lib_available: bool` を追加した。`false` の場合、`classify_all`
が Fast path と判定した名前も無条件で Fallback に格下げする。
`thaw-cli` は `--use`（レジストリ経由）では `package.native_lib.is_some()`
をそのまま渡す一方、`--bridge`（手動経路）では常に `true` を渡す --
`--bridge` はユーザーが自分で `--link` を用意する前提の経路であり、
この判断はユーザーに委ねられたまま変えていない。

## 5. Marshal/Unmarshal コード生成

Fast path に分類された関数は、[HirExpr::FfiCall](../../crates/thaw-hir/src/lib.rs)
と `thaw_hir::FfiSignature`（すでに雛形として存在していた型）を
そのまま使う。今回の実装で実際に構築されるようになった。

**重要な設計判断**: 今回実装する垂直スライスでは、外部ネイティブ
関数の ABI は **Thaw 自身の内部表現とそのまま一致する**ことを前提に
している（`Str` = null終端 `i8*`、`Array(F64)` = `[i64 len][f64...]`
という thaw-hir/hir_codegen 独自のレイアウトそのまま）。これは
「Thaw で書かれた別のネイティブライブラリを呼ぶ」場合は正しいが、
**実際の npm パッケージが期待する C ABI とは通常一致しない**
（例: 多くの C 言語 API は配列を `(ptr, len)` の2引数に分けて渡す。
文字列も UTF-8 とは限らないし長さ境界が別引数のこともある）。

したがって、本物の npm パッケージ向けの Marshal コード生成は
次のステップとして別途必要になる：

- `Array(F64)` → 呼び出し直前に `(f64*, i64 len)` の2引数に展開する
  アダプタ命令列を挟む（Thaw 内部の `[len][elements...]` バッファから
  ヘッダを読み、要素部分の生ポインタと長さを2つの引数として渡す）。
- `Str` はそのまま渡せることが多い（C の慣習と一致）が、長さ境界を
  別引数で要求する API（`(const char*, size_t)` 形式）には同様の
  展開が要る。
- `Object` は対象ライブラリの構造体レイアウトと Thaw のレイアウトが
  一致する保証がないため、フィールドごとに個別の代入命令列を生成する
  必要がある（単純な `memcpy` 一発では済まない）。

このレイヤーは `.d.ts` だけからは分からない情報（実際の C ABI の
呼び出し規約の詳細）を必要とすることが多く、パッケージごとの
追加メタデータ（将来のレジストリ側で補完する想定）が要る可能性が
高い。**この汎用アダプタ生成は今回のスコープ外**、今回は
「Thaw の内部表現をそのまま期待するネイティブ関数」を呼ぶところ
までを実装する。

## 6. 実装した垂直スライス

今回のセッションで実際にコード化したのは以下：

- `thaw-bridge` crate: `.d.ts` ソースをパースし（thaw-parser 経由）、
  トップレベルの `declare function` シグネチャを抽出、4章の分類器で
  Fast path / Fallback を判定する。
- thaw-hir: 本体を持たない関数宣言（`declare function foo(...): T;`、
  または通常の `.ts` ファイル中に直接書かれた同じ構文）を「ambient
  関数」として認識し、`HirProgram.extern_functions: Vec<FfiSignature>`
  に集める。ambient 関数への呼び出しは `HirExpr::Call` ではなく
  `HirExpr::FfiCall` にローワリングされる。
- thaw-llvm/hir_codegen: `extern_functions` の各シグネチャを
  `Linkage::External` な LLVM 関数として宣言し、`FfiCall` はその
  宣言への通常の `call` 命令にコンパイルする。

つまり、ユーザーは自分の `.ts` ファイルの中で

```ts
declare function native_add(a: number, b: number): number;

function main(): void {
  console.log(native_add(2, 3));
}
```

のように書け、`native_add` の実体は `thaw build` がリンクする
別の静的ライブラリ（今は手動で用意する必要がある）が提供する。
「`.d.ts` を読んで npm パッケージ全体を解決し、レジストリから
`.so` を取得してリンクする」という自動化（thaw-registry の仕事）は
まだ存在しない -- 今回のスライスは、その自動化が最終的に生成する
はずの**コンパイラ側の受け口**（ambient 宣言 → 実FFI呼び出し）を
先に作った、という位置づけ。

## 7. QuickJS-NG 統合（インターフェース設計のみ、未実装）

Fallback path は、対象パッケージの JS 本体を QuickJS-NG
（[rquickjs](https://crates.io/crates/rquickjs)）上で実行し、
Fast path 対象外の値だけ動的にやり取りする。V1 として想定する
最小インターフェース：

```rust
// thaw-bridge が生成する呼び出し側コード（擬似コード）
extern "C" fn thaw_dynamic_call(
    module: *const c_char,   // "lodash" のようなパッケージ名
    func: *const c_char,     // 呼び出す関数名
    args_json: *const c_char, // 引数を JSON エンコードしたもの
) -> *const c_char;           // 結果を JSON エンコードして返す
```

- 引数/戻り値は `thaw-std` の `Json`（[thaw-std/src/json.rs](../../crates/thaw-std/src/json.rs)）
  としてやり取りする -- 既存の `HirType::Json` とその
  `JsonGet`/`JsonAsNumber`等のアクセサ群がそのまま使える。
- QuickJS 側のグローバルスコープに、対象パッケージの JS ソースを
  一度だけロードしておき、呼び出しのたびに `func` を検索して呼ぶ
  （モジュール解決は Node.js 由来の `require`/`import` 解決を
  QuickJS-NG 上に自前実装する必要があり、これ自体が別途大きい課題）。
- Promise を返す関数は、V1 の async/await 設計
  （[docs/design/async-await.md](async-await.md)）と同様、V1では
  「JS 側の Promise が解決するまで同期的にブロックする」実装で
  済ませられる可能性が高い（QuickJS-NG は `JS_ExecutePendingJob` を
  ポーリングすれば同一スレッドで Promise を進行させられるため、
  「解決するまでポーリングし続ける」だけで動く -- 本物の
  ノンブロッキング統合は async/await V2 と合わせて設計する）。

### 未解決の論点

- QuickJS-NG のモジュール解決（`require`/ESM）をどこまで自前実装するか。
- ネイティブアドオン（`.node` ファイル）に依存する npm パッケージは
  QuickJS-NG 上でも動かせない（トップレベル設計ドキュメントで
  明示的にスコープ外とされている: 9章参照）。
- レジストリ（`.so` のビルド・キャッシュ・配布）との連携方法。

## 8. 今回やらなかったこと（意図的なスコープ外）

以下のうち3項目は、この節を書いた時点では未着手だったが、その後の
セッションで実装済みになっている（このドキュメント自体は当時のまま
残してあるが、リンク先が最新の状態）:

- ~~`interface` 宣言のサポート~~ -- 実装済み（`03b2e96`〜`11966fb`、
  named object types・nested fields・extends・ジェネリックの
  オンザスポット単相化まで）。
- ~~QuickJS-NG の実装統合~~ -- 実装済み（`328b4d6`）。7章の
  インターフェース設計がそのまま実装に落ちた。
- ~~thaw-registry~~ -- 実装済み、[registry.md](registry.md) 参照。
  「`.so` の解決・キャッシュ・配布」というより「`.d.ts`/JS 実装の
  自動取り込み」が実際の中心的な価値になった。

以下はまだ残っているもの:

- 実際の npm パッケージの `.d.ts` を読み込んでの動作確認は
  Fast path 自体には未実施のまま（Fast path 用の `native.a` を
  自動生成する npm パッケージが実質存在しないため -- registry.md
  18章参照）。Fallback 経路は実物パッケージで広く検証済み。
- 5章で述べた、外部ライブラリの実際の C ABI 規約（`(ptr, len)` 分割
  など）に合わせた Marshal アダプタ生成 -- **`number[]` パラメータの
  `(const double*, int64_t len)` 展開と、object パラメータのフィールド
  単位への展開は実装済み**（`thaw-llvm::hir_codegen::ffi_param_types`/
  `compile_ffi_call`、手書きの実 C 関数とリンクして検証）。戻り値側の
  マーシャリング（`Array`/`Object` を返す C 関数への対応）と、文字列の
  `(ptr, len)` 分割規約（`string` はそのまま `const char*` として渡す
  規約のみ対応）は引き続き未対応。
# Result ABI metadata

The manual bridge path accepts a separate, versioned JSON document through
`--ffi-metadata`. Version 1 maps ambient function symbols to an `errorAbi` of
either `direct` (the default) or `thaw-result`. For a non-void declared return
type `T`, `thaw-result` declares the native symbol as returning the target C ABI
equivalent of `struct { T value; const char *error; }`. LLVM extracts the error
pointer into Thaw's pending-exception channel before exposing the value, so the
existing `try/catch/finally` implementation handles native failures unchanged.

Version 2 keeps that ABI field and adds `returnOwnership` and `errorOwnership`.
Each defaults to `borrowed`; `owned` requires `destroy`/`returnDestroy` for the
return channel or `errorDestroy` for the error channel; `arena-copy` accepts an
optional destructor. For non-null C strings, LLVM copies `strlen(ptr) + 1`
bytes into `thaw_arena_alloc` before invoking the destructor. The copy therefore
survives native deallocation and remains valid through return and catch/finally
paths. Null pointers are preserved and never passed to a destructor. Ownership
metadata is currently limited to `string` returns and the thaw-result error
string; unsupported type/ownership combinations are compilation errors.

Metadata is deliberately separate from `.d.ts`: TypeScript declarations do not
describe C ownership or error conventions. Unknown versions, ABI spellings, or
ambient symbols are rejected instead of silently assuming a calling convention.
Version 1 remains backward-compatible and implies borrowed ownership. Void
thaw-result values and aggregate ownership remain future extensions.
