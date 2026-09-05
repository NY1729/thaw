# npm相互運用: 2026-09 ギャップ調査と実装記録

2026-09-03。前セッションで発見した5件の未対応npmパッケージ挙動について、実装に入る前に規模を調べた結果をまとめる。

## 追補（2026-09-06）

この文書の「現状」「対象外」は調査時点の記録であり、後続実装で一部が解消された。現在はFallbackクラスの
`new`、通常の`JsValue`プロパティ読み出し、メソッド/プロパティの再帰的チェーン、namespace/re-exportを含む
宣言flatten、呼び出し引数の形に基づくFallback関数のoverload選択まで実装済みである。zod、hono、drizzle、
lodash、dayjs、uuidは実パッケージ統合テストで追跡している。

後続実装により、uuidの`NIL`/`MAX`やmimeの既製singletonのような、packageが直接公開する型付き
非callable値exportもmodule初期化後に一度取得して公開できる。chalkに必要な一般的なプロパティ読み出しとチェーンの基盤は実装済みだが、chalk
自体の互換性は実パッケージテストが追加されるまで未検証として扱う。また、真に動的な`loadScript`、
`callDynamic`、opaqueな`JsValue`操作は、汎用JS runtimeを再実装せずQuickJSに残す設計である。

## 調査結果サマリ

| 項目 | 規模 | 独立性 |
|---|---|---|
| mime (`export = Mime` が別ファイルの class import) | 中 | **dayjs/chalkと同じ大きな欠落機能に依存** |
| camelcase (Union型の動的呼び出し引数のJSON化) | 中 | 独立、既存コードへの局所的な追加 |
| date-fns (Dateオブジェクトの境界マーシャリング) | 大 | 独立、型システム+コード生成+JS側の3箇所 |
| chalk (チェーン可能なProxy風API) | 特大 | **mime/dayjsと同じ欠落機能 + さらに大きい再帰的機能が別途必要** |
| zod (スキーマビルダー、深いジェネリッククラス階層) | 特大 | 事実上フルスケールのTS型システム実装が必要 |

## 重要な発見: 3件が同じ根本原因に収束する

mime (`mime.getType(...)`)、dayjs (`dayjs().format(...)`)、chalk (`chalk.red.bold(...)`) はいずれも
「**Fallback（QuickJS経由、ネイティブアドオンを使わない）パッケージが返すクラスインスタンスのメソッドを呼び出す**」
という同一の機能を必要としている。

現在のコードを確認した限り、クラスのメソッドブリッジ（`DtsClass`/`DtsMethod`→ネイティブ呼び出し生成）は
`crates/thaw-cli/src/registry_integration/shims.rs` の

```rust
if pkg.native_addon.is_some() && pkg.bundle_js.is_none() {
    shim.push_str(&generate_native_addon_shim(&pkg.functions, qualified));
} else {
    shim.push_str(&generate_shim(&pkg.functions, native_lib_available, qualified));
}
```

という分岐で、**ネイティブアドオン（`.node`バイナリ）を持つパッケージにしか適用されない**。
純粋なJS実装（`bundle.js`のみ）のパッケージについては `pkg.classes`（`DtsClass`）が一切ブリッジされず、
factory関数が返すクラスインスタンスは常に不透明な`JsValue`ハンドルのまま（前回の`dayjs`修正はここで留まっている）。

つまり「mimeを直す」「dayjsのメソッドを呼べるようにする」「chalkの`.red`を実装する」は、
表面上は別々の要望に見えるが、実装すべきものは1つ:
**「QuickJS-NG経由でクラスインスタンスのメソッド/プロパティを動的に呼び出す仕組み」**（ネイティブアドオン版の
`generate_native_addon_shim`/`compile_typed_napi_setter`系に相当するものを、Fallback/`callDynamic`版として作る）。

## 実装方針（優先度順）

### 1. camelcase: Union型を動的呼び出し引数としてJSON化できるようにする（独立・中規模）

**現状**: `crates/thaw-llvm/src/hir_codegen/json_bridge.rs` の配列要素シリアライズ（`compile_native_array_to_json_with_undefined`系、`console.log`と動的呼び出し引数マーシャリングの両方から共有されている）が `HirType::Union` を扱えず `Err` になる。

**やること**:
- 配列要素/動的引数のJSON化ヘルパーに `HirType::Union(elements)` のケースを追加。
  - Union値の実行時タグ（`UnionTag`相当のLLVM表現）を読み、該当メンバー型で再帰的に同じシリアライズ処理を呼ぶ（LLVMの分岐/switchを生成）。
- `crates/thaw-hir/src/lower/invocations/static_builtins.rs` の `json_convertible_native_type` にも `HirType::Union(_)` を追加し、`coerce_to_declared`のJson化経路（`wrap_native_value_as_json`）でもUnion値を扱えるようにする。
- 影響範囲は「JSON化」の1レイヤーに閉じている。呼び出し先のクラス/インターフェース機能とは独立。

**検証**: `camelCase(input: string | readonly string[], options?)` を実際に呼び、`string`側・`string[]`側の両方が正しくJSON化されることを確認。

### 2. mime: `import Type = require(path)` を型解決のために追いかける（独立・中規模、ただし機能全体が動くには#4が必要）

**現状**: `import Mime = require("./Mime")` は semverの時に実装した「値の再エクスポート追跡」(`import_equals_targets`)では拾えるが、`declare const mime: Mime;` の `Mime` は**型としての参照**であり、関数ではなくクラス。`Mime`の実体は別ファイルにあり、現在のトリプルスラッシュ参照インライン化・`export_assignment_interface_name`はいずれも「同一ファイル内」を前提にしている。

**やること**:
- `crates/thaw-registry/src/registry/install.rs` の flatten処理で、`import Name = require(path)` を**値の再エクスポート**だけでなく、**型参照の解決**にも使う: `declare const x: Name;` の `Name` が import-equalsで解決できる場合、そのファイル全体（`declare class Name {...}`含む）をインライン化する。
- これは概念的にはトリプルスラッシュ参照インライン化と同じパターン（「参照先ファイルを読み込んで合成する」）の横展開であり、パーサ/AST的な難度は高くない。
- **ただし**: これでMime.d.tsの`class Mime {...}`が package.d.ts に取り込まれても、`parse_dts_classes`がそれを拾い、実際に**呼び出し可能にする**には#4（Fallbackクラスのメソッド呼び出し機構）が必要。#4が無いと「解析はできるが呼べない」状態で終わる。

### 3. date-fns: ネイティブDate値をFallback呼び出し境界で本物のJS Dateとして受け渡す（独立・大規模）

**現状**: `new Date(...)`はthaw内部で単なる`F64`（タイムスタンプ）として扱われる（`HirType`に`Date`という型は存在しない）。Fallback呼び出しの引数マーシャリングはJSON経由であり、素の数値としてJSON化されると、受け取り側（date-fns自身の`toDate()`等）は`instanceof Date`ではないため無効な日付として扱う。

**やること（3箇所にまたがる）**:
1. **型システム**: 「これはDateを表すF64である」という情報をどこかで保持する必要がある。案:
   - (a) `HirType`に新バリアント`Date`を追加し、`.d.ts`の`Date`型参照を`Native(HirType::Date)`として分類する、または
   - (b) 呼び出しの引数コンテキストで「宣言側の型が`Date`」という情報だけを使い、`F64`値を渡す直前に特別なタグ付きJSON（例: `{"__thaw_date__": <timestamp>}`）に変換する。
   - (a)の方が筋が良いが影響範囲が広い（`HirType`を消費する全箇所への影響）。(b)は影響を呼び出し境界に閉じ込められるため、まずは(b)で様子を見るのが安全。
2. **コード生成**: 引数マーシャリング時、宣言型が`Date`であるF64値を上記の特別な形にラップするコード生成を追加。
3. **JS側呼び出し規約**: 生成されるshim/`callDynamic`のグルーコードで、受け取った`{"__thaw_date__": N}`形式を`new Date(N)`に復元してから実関数に渡す一手間を追加。戻り値がDateの場合も同様に`.getTime()`してタグ付きで返す。

**検証**: `format(new Date(2024,0,15), "yyyy-MM-dd")`が正しい文字列を返すこと。

### 4. Fallbackクラスのメソッド呼び出し機構（新規・大規模、mime/dayjs/chalkの共通基盤）

**現状**: 前述の通り、`pkg.classes`はネイティブアドオンパッケージでしか使われない。

**やること（概略、詳細設計は着手前にもう一段掘り下げが必要）**:
- `DtsClass`/`DtsMethod`（`thaw-bridge`側で既にパース済み）を、ネイティブアドオン版と並行して**Fallback版**でもコード生成する新パス `generate_fallback_class_shim` を追加。
- コンストラクタ呼び出し・メソッド呼び出しはいずれも`callDynamic`系（既存のFallback関数と同じJSON経由呼び出し）を使い、「インスタンスをどう識別してJS側の実オブジェクトに結びつけるか」を設計する必要がある(候補: QuickJS側でインスタンスをハンドルテーブルに保持し、Rust側は整数ハンドルだけ持つ — 既存の`JsValue`/`callDynamicValueHandle`の仕組みに近い)。
- メソッドの引数/戻り値の型分類・JSON化は既存のFallback関数と同じ仕組みを再利用できる。

**この項目は他の3件（mime完全対応・dayjsのメソッド呼び出し・chalk）の土台になるため、優先度は高いが、着手前に既存の`JsValue`ハンドル機構(`callDynamicValueHandle`等)を深く読み込んでから設計を固める必要がある。**

### 5. chalk: チェーン可能なAPI + ESM `export default <plain value>`

**現状**: 二重の欠落。(a) `export default chalk;` で`chalk`がプレーンな値（関数宣言ではない）のケースを`commonjs_export_name`が解決できない。(b) `ChalkInstance`インターフェースは「呼び出し可能」かつ「`.red`/`.bold`等のプロパティが再帰的に同じ`ChalkInstance`を返す」という、単純なクラスメソッド呼び出しを超えた**プロパティアクセスチェーン**が必要。

**評価**: (a)は#4と組み合わせれば解決できる可能性がある。(b)は#4の機構を「メソッド呼び出し」だけでなく「プロパティアクセスが新しいハンドルを返す」形に一般化する必要があり、#4の完成を前提にしても追加の設計が要る。**費用対効果を考えると、#4完成後に改めて評価すべき項目**であり、今回のスコープには含めない。

### 6. zod: スコープ外として明確に除外を提案

zodのAPIは「ジェネリックなスキーマクラス階層 + メソッドチェーン + 実行時バリデーション」であり、
`z.object({...})`一つを取っても、オブジェクトリテラル引数の型をジェネリックに推論し、結果として
新しいスキーマ「型」を返すという、TypeScriptの構造的型システムの相当部分を再実装するに等しい。
**このプロジェクトの現在の型ブリッジ機構の延長では対応できず、対応するなら独立した大規模プロジェクトになる。**
今回は実装対象から除外することを提案する。

## 提案する実装順序

1. camelcase（Union JSON化) — 独立・中規模、すぐ着手可能
2. date-fns（Date境界マーシャリング) — 独立・大規模だが設計は上記で固まっている
3. mime向けの型import解決（トリプルスラッシュの横展開） — 独立・中規模、ただし完全に動くのは#4後
4. Fallbackクラスのメソッド呼び出し機構 — 大規模、mime/dayjsを完全に動かす土台。着手前に既存`JsValue`/`callDynamicValueHandle`機構をさらに調査し、設計を1段深める。
5. chalk — #4完成後に再評価。今回のスコープからは外す。
6. zod — 対象外。

## 実装結果（2026-09-03）

1〜4を実装・コミット済み（各コミットの本文に詳細）。

- **#1 camelcase**: 想定通り実装。Union型のJSON化を配列要素・オブジェクトフィールドの両階層に追加。
- **#2 date-fns**: `HirType::Date`の新設ではなく、既存の`date_object_type`（`{timestamp: F64}`のObject）をそのまま`.d.ts`の`Date`型分類に流用する方針に着地（タグ付きJSON封筒案より低リスク）。QuickJS-NG側で`Date.prototype.toJSON`オーバーライドと`JSON.parse`のreviverを追加し、`{"timestamp": N}`を境界での共通ワイヤ形式にした。実装途中で3件の独立したバグ（generic関数の非bare型パラメータ戻り値が無条件`JsValue`になる、生成される宣言がoptional trailing paramを持つ非genericなambient関数になった場合のarity不整合、`Json`にフォールバックした型パラメータへの実引数マッチングがConcrete比較で失敗する、`resolve_value_impl`が`undefined`結果でエラーになる)を発見・修正。**既知の副作用**: date-fns/dayjsのローカル時刻ベースのformatトークン（`HH`等）はホストのOSタイムゾーンに依存する（QuickJS-NGのDateはUTC専用化されていない）。日付計算・UTCアクセサ自体には影響なし。
- **#3 mime**: `import Mime = require("./Mime")`のような、値の型が別ファイルにあるケースをインライン化。mimeの`Mime`クラス本体はflatten後の`package.d.ts`に載るようになったが、`mime`という値自体は「関数ではなく既製のシングルトンオブジェクト」であるため、素の`JsValue`としてすら現状インポートできない（`mime has no export named default`）。この「シングルトン値エクスポートの公開」は#4の範囲にも含めず、未解決のまま。
- **#4 Fallbackクラスのメソッド呼び出し**: 当初想定より掘り下げたところ、実際に必要だったのは「QuickJS-NG版のtypedメソッド呼び出しコード生成」（小〜中規模、既存のnapiクラス機構をbackend分岐で共有）に加えて、**「Fallbackのファクトリ関数の戻り値をどのクラスのインスタンスとして追跡するか」**（`function_return_named_types`の新設、`declare namespace`内のクラス抽出漏れの修正、`class_methods.rs`への`FactoryClassRewrite`追跡の追加）という、事前调査時には見えていなかった追加の一段だった。ユーザーに詳細を報告した上で「リスクを承知で実装する」の判断を得て実施。
  - **できるようになったこと**: `dayjs(...).format(...)`/`.year()`/`.month()`/`.date()`/`.isValid()`など、Fallbackのファクトリ関数が返すクラスインスタンスへの**インスタンスメソッド呼び出し**が実際のdayjsパッケージで動作する。
  - **後続実装で解消**: Fallbackクラスに対する`new ClassName(...)`、通常の`JsValue`プロパティ読み出し、メソッド/プロパティの再帰的チェーン。
  - **後続実装で解消**: mimeのような「関数ではなくpackageが直接エクスポートする既製のシングルトン値」と、uuidの文字列定数を含む型付き非callable値export。staticメソッドとgetter/setterの対応状況は、この文書では保証対象にしない。
  - **chalk**: 必要な動的チェーンの基盤は後続実装済み。ただし実パッケージによる互換性確認は未実施。
