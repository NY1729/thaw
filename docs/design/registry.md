# thaw-registry 設計ドキュメント（V1: ローカルディレクトリ）

- ステータス: 設計 + 最初の垂直スライス実装
- 前提: [thaw-bridge](../../crates/thaw-bridge), [docs/design/bridge.md](bridge.md) の分類/シム生成機構を前提にする
- 位置づけ: [[project_thaw_overview]] が「Thaw の本質的価値」と位置づける部分そのもの。bridge.md 8章・7章で「未実装」として送られていた `thaw-registry` を、最小の形で実装する。

## 1. これまでの状態と、埋めるギャップ

bridge.md までの実装で、`.d.ts` を渡せば Fast path / Fallback を自動判定し、
呼び出し可能な TS シムを生成する（`thaw_bridge::generate_shim`）ところまでは
できていた。ただし、ユーザーから見るとまだ3つの手作業が残っていた：

1. `--bridge <path.d.ts>` を毎回手で指定する
2. Fast path のシンボルを提供する静的ライブラリを `--link` で手で指定する
3. Fallback 関数を使う前に、パッケージの実 JS ソースを `loadScript(...)`
   で自分のコード中に手で書いて一度ロードする

「パッケージ名を1つ指定するだけで、この3つが自動化される」ところまでを
今回のスコープとする。**ネットワーク経由の取得・バージョン解決・
ネイティブライブラリのビルドは意図的にスコープ外**（7章）。

## 2. V1 レジストリの形：ただのローカルディレクトリ

```
<registry-dir>/<package-name>/
  package.d.ts   (必須)  -- thaw-bridge の parse_dts/classify に渡す
  native.a       (任意)  -- Fast path 関数の実体を提供する静的ライブラリ
  bundle.js      (任意)  -- Fallback 関数の実体となる JS ソース
```

`package.d.ts` だけが必須。片方しか要らないパッケージもある
（純粋な Fast path パッケージは `bundle.js` 不要、純粋な Fallback
パッケージは `native.a` 不要）。

`thaw-registry` crate（[crates/thaw-registry](../../crates/thaw-registry))
はこのディレクトリ規約を読むだけの薄い層：

```rust
pub struct ResolvedPackage {
    pub name: String,
    pub dts_source: String,
    pub native_lib: Option<PathBuf>,
    pub bundle_js: Option<String>,
}

pub fn resolve(registry_dir: &Path, name: &str) -> Result<ResolvedPackage, String>;
```

## 3. CLI: `--registry` / `--use`

```
thaw build app.ts --use left-pad --use is-odd
thaw build app.ts --registry ./vendor --use left-pad
```

- `--registry <dir>`: レジストリのルート（デフォルト `./thaw_modules`）
- `--use <package>`: レジストリから解決するパッケージ名（複数指定可）

thaw-cli はここで解決したパッケージごとに:

1. `package.d.ts` を `thaw_bridge::parse_dts` + `generate_shim` に通し、
   結果のシムをユーザーソースの前に結合する（`--bridge` と同じ仕組み）。
2. `native.a` があればリンク対象に自動的に加える（`--link` を書く必要が
   なくなる）。
3. `bundle.js` があれば内容を集約し、4章の `__thaw_module_init` に
   まとめる。

既存の `--bridge`/`--link` はそのまま残す（レジストリに登録していない
一発物の `.d.ts`/ライブラリを試す手動経路として引き続き有効）。

## 4. モジュール自動初期化: `__thaw_module_init`

「Fallback 関数を使う前に `loadScript` を一度呼ぶ」という手順を自動化
するには、ユーザーの `main`/`handler` の**先頭で**何かを実行できる
仕組みが要る。Thaw にはまだトップレベル文やモジュールシステムがないため、
新しい構文は増やさず、**特別扱いする関数名**で済ませることにした：

- `thaw-bridge::generate_module_init(&[(name, js_source)]) -> String` が
  `function __thaw_module_init(): void { loadScript("..."); ... }` という
  ふつうの Thaw 関数を生成する（複数パッケージ分の `loadScript` 呼び出しを
  `--use` で指定した順に並べる）。
- thaw-llvm の `compile_program` は、`main`/`handler` どちらの
  エントリポイントを合成する場合も、実際のユーザーコードを呼ぶ**前**に
  `__thaw_module_init` が定義されていればそれを呼ぶ
  （`MODULE_INIT_SYMBOL`/`call_module_init_if_present`、
  [hir_codegen.rs](../../crates/thaw-llvm/src/hir_codegen.rs)）。
  定義されていなければ何もしない（`bundle.js` を持つパッケージを
  1つも `--use` していないプログラムでは、生成すらされない）。

これにより `__thaw_module_init` は HIR/codegen にとって「ただの関数」
のまま扱える -- 新しい `HirStmt`/`HirExpr` もパーサー変更も不要。
唯一の特別扱いは「存在すれば呼ぶタイミング」だけ。

## 5. 検証した垂直スライス

- `thaw-registry`: ディレクトリ規約の解決（全ファイルあり/必須のみ/
  `.d.ts` 欠落エラー/パッケージ自体が存在しないエラー、の4パターンを
  ユニットテストで確認）。
- `thaw-bridge::generate_module_init`: 生成される `loadScript` 呼び出しの
  順序、JS ソース中の `"`/`\`/改行のエスケープ、そして生成コードが
  実際に `thaw_parser`→`thaw_hir` でパース/lowering できることを
  round-trip テストで確認（bridge.md の `generate_shim` と同じ方針）。
- thaw-llvm: `__thaw_module_init` が `main` エントリでも `handler`
  （Lambda）エントリでも、ユーザーコードより先に実行されることを、
  実際にコンパイル・リンクしたネイティブバイナリを実行して確認
  （Lambda 側はモック Runtime API サーバーに対する実際の HTTP
  往復で確認 -- 既存の Lambda テストと同じプロトコル）。
- thaw-cli: `--registry`/`--use` を実際に使い、Fast path 関数
  （`native.a` 自動リンク）と Fallback 関数（`bundle.js` 自動ロード、
  ユーザーコードに `loadScript` 記述なし）を1つのプログラムに混在させて
  ビルド・実行し、両方とも正しい出力を得た。

## 6. Fallback 側の実行を実物の npm パッケージで検証、CommonJS 対応

上記5章の検証は自作の `bundle.js`（素のグローバル関数宣言）でしか
行っていなかった。実際に `npm install left-pad slugify is-odd` して
本物の公開済み JS をそのまま `loadScript` に流し込んだところ、
`ReferenceError: module is not defined` で即座に落ちた -- 実際の npm
パッケージはほぼ例外なく CommonJS（`module.exports = ...`）か UMD
（`typeof exports === 'object'` 分岐）で書かれており、QuickJS-NG の
素のグローバルスコープには `module`/`exports`/`require` が存在しない
ため。bridge.md 7章が「別途大きい課題」として保留していたモジュール
解決問題の、最小限の一角を埋めた：

- `thaw_bridge::wrap_as_commonjs_module`: パッケージの JS ソースを
  `loadScript` に渡す前に、`module`/`exports`/`require` を**グローバル
  変数として**定義してから、ソース自体は一切ラップせずそのまま
  トップレベルで実行する（関数スコープで囲むと、素のグローバル関数
  宣言に依存する既存の自作 `bundle.js` が壊れるため、意図的にこの形）。
  実行後、`module.exports` がオブジェクトならその各プロパティを、
  関数なら `.d.ts` から分かっている Fallback 関数名
  （`ModuleBundle::fallback_names`）でグローバルに束縛し直す。
- `require` はスタブで、呼ばれた瞬間に例外を投げる。実際のパッケージ間
  依存解決（`is-number` を要求する `is-odd` のようなケース）は
  引き続きスコープ外だが、以前は `module is not defined` という
  無関係なエラーで落ちていたのが、`require('is-number') is not
  supported in the Fallback path yet` という実際の原因を示す
  エラーになった（thaw-quickjs 側の `thaw_js_load` も、投げられた
  例外の実メッセージ（`describe_exception`）を使うよう修正 --
  以前は `rquickjs::Error::Exception` の汎用プレースホルダしか
  表示されなかった）。

**検証**: 依存を持たない実物の npm パッケージ（`left-pad`、`slugify`）
を無改造のまま `--use` 経由でロード・実行し、正しい結果を得た
（`leftPad("5", 4, "0")` → `"0005"`、`slugify("Hello World!")` →
`"Hello-World!"`）。依存を持つパッケージ（`is-odd` が要求する
`is-number`）は、`require` スタブにより読み込み時点で明確なエラーに
なることを確認した -- これはバグではなく、上記の通り意図的な
未対応領域。

## 7. `thaw registry add`: 自動取り込み

ここまでの検証は全て、私が `npm install` して `package.d.ts`/`bundle.js`
を手でコピーして `thaw_modules/` に置く、という手動キュレーションの上に
成り立っていた。差別化要因は「自動化」のはずなのに、自動化された部分が
まだ何もない状態だった。`thaw registry add <package>` で、この手作業を
実際に自動化した：

1. 使い捨てのスクラッチディレクトリに `npm install --prefix <scratch>
   --ignore-scripts <package>` でパッケージを取得する。
   `--ignore-scripts` は明示的な安全策 -- 無人で任意の第三者パッケージの
   `postinstall` を実行するのは避ける。
2. 取得した `package.json` を読み、型定義の場所を特定する:
   `types` フィールド → `typings` フィールド → （どちらもなければ）
   `index.d.ts` の存在確認、の順。
3. どれも見つからなければ、DefinitelyTyped の対応パッケージ
   （`@types/<package>`、スコープ付きなら `@types/<scope>__<name>`）を
   追加で `npm install` し、そちらに同じ手順（2）を適用する。
   `@types/*` パッケージ自体は実行時 JS を持たない（`main` が空）ため、
   ここから採用するのは型定義だけ -- 実行される JS は常に元の
   `package` 自身のものを使う。それも失敗すれば（`@types/<package>`
   が存在しない、または型定義が見つからない）、原因を明示した
   エラーで失敗する。
4. `main` フィールド（なければ `index.js`）を JS エントリポイントとする。
   実際の npm パッケージ（`ms`）でこの手順が失敗するケースを見つけて
   修正済み: `main` の値をそのままファイルパスとして読もうとすると、
   `"main": "./index"`（拡張子なし、Node が require 時に自動補完する
   前提）や `"main": "./lib"`（ディレクトリを指し、実体は
   `lib/index.js`）で失敗する。`resolve_main_js_path` が、値そのまま
   → `.js` 拡張子を付与 → `/index.js` を末尾に付与、の順で実在する
   ファイルを探す（Node の解決アルゴリズムの一部だけを真似た簡易版）。
5. 両ファイルの内容を読み、`registry_dir/<package>/package.d.ts` /
   `bundle.js` として書き込む（スクラッチディレクトリは処理後に削除）。

`native.a` は生成しない -- ネイティブアドオンのビルドパイプラインは
8章の通り引き続きスコープ外で、`thaw registry add` は純粋な JS
パッケージだけを対象にする。

**検証**: 空のレジストリディレクトリから `thaw registry add left-pad`
を実行し、生成された `package.d.ts`/`bundle.js`（実際に npm から
取得した無改造の内容）を確認した上で、`thaw build --use left-pad`
で実際にビルド・実行し、正しい出力（`00007`）を得た -- 手作業による
ファイル配置を一切介さない、初めての完全自動フローの確認。型定義を
自前でバンドルしていない `is-odd`（3章の検証時点では「非対応」だった
ケース）は、`@types/is-odd`（実在する）への自動フォールバックにより
今は正しく `package.d.ts`（型は `@types/is-odd` 由来、JS は元の
`is-odd` 自身）を生成できることを確認した。そのまま `thaw build
--use is-odd` を実行すると、`is-odd` 自身が要求する
`require('is-number')` によって（6章の通り、意図的にスコープ外の
依存解決問題として）明確なエラーで失敗することも確認した -- 「型定義が
見つからない」問題と「パッケージ間依存が解決できない」問題は別物で、
今回直したのは前者だけ。この `add` ステップ自体はネットワーク（npm
レジストリ）に依存するため、自動テストスイート（オフラインでも
動く必要がある）には含めず、手動検証のみで確認している --
`find_own_dts`/`types_package_name`/`resolve_module_path`（型定義/JS
エントリポイントの場所特定ロジック）はネットワーク非依存なので
ユニットテストで自動検証している。

`main` フィールド解決の修正と、bridge.md 4.3節のオーバーロード対応
（`classify_all`）を合わせて実際の `ms` パッケージで再検証: `thaw
registry add ms` が成功し（修正前は `"main": "./index"` で失敗）、
`thaw build --use ms` で2種類の呼び出し形（`ms(60000)` →
`"1m"`、`ms("2 days")` → `172800000`）が両方とも正しく動くことを
確認した -- 生成されたシムに `ms` という名前の宣言が正確に1つだけ
含まれることも合わせて確認済み。

## 8. 自分自身の複数ファイルへの分割: 簡易 CommonJS バンドラ

7章で確立した自動取り込みを実際の `qs` パッケージで検証したところ、
新しい種類の失敗が見つかった。`qs` の `main`（`lib/index.js`）は

```js
var stringify = require('./stringify');
var parse = require('./parse');
var formats = require('./formats');
```

という、**外部パッケージへの依存ではなく自分自身の内部ファイルへの
相対 `require`** から始まる。それまでの `bundle.js` はパッケージの
`main` 1ファイルだけをコピーしたものだったので、6章の `require`
スタブ（無条件に例外を投げる）がこれをそのまま潰してしまう -- 原因は
外部依存ではなく、単にファイルが複数に分かれているだけなのに、である。
ある程度の規模のパッケージが自分のコードを複数ファイルに分けるのは
ごく普通のことなので、これは `is-odd`（7章、真の外部依存）より
広い範囲に影響する制限だった。

対策として、`add` の時点（パッケージ全体がまだスクラッチディレクトリに
残っている間）に、`main` から到達可能な相対 `require` を再帰的に
辿って**同一パッケージ内のファイルだけ**を1つの JS にバンドルする
`bundle_commonjs_package` を実装した:

1. `main` から開始し、各ファイルを thaw-parser の JavaScript mode で
   構文木にして、literal `require()`、静的 `import`、`export ... from`、
   `export * from`、literal `import()` を収集する。コメント・文字列・
   member call・動的 `require(variable)` は依存と誤認しない。相対依存と
   bare package依存を同じグラフへ載せる。
2. 各相対 require の相手先を、既存の `resolve_module_path`（7章、
   元々は `main` フィールド解決用だった関数を汎用化）で解決し、
   まだ見ていないファイルならキューに積む。解決できない場合
   （対応外のnative moduleなど）はそのファイルの依存マップに
   含めず、実行時に外部 require スタブへフォールスルーさせる --
   バンドル全体を中断させない。
3. 集めた各ファイルを `function(module, exports, require) { ... }`
   という素の CommonJS ファクトリとしてオブジェクトリテラルに並べ、
   ファイルごとに「このファイルの相対 require 文字列 → 解決済みの
   グローバルキー」という対応表も**静的に**埋め込む（実行時にパス解決
   のロジックを JS 側に持たせる必要が一切ない）。最後に
   `module.exports = __thaw_bundle_require("<mainのキー>");` で
   エントリポイントを起動する。
4. こうして生成された1つの JS 文字列は、6章の `wrap_as_commonjs_module`
   にとって「複数ファイルのバンドルである」ことを一切意識させない --
   相変わらず「`module`/`exports`/`require` をグローバルに用意して
   1つの JS を実行する」という既存の契約のままで動く。

外部パッケージへの `require`（バンドル内で解決できなかったもの）は
そのまま `wrap_as_commonjs_module` が用意したグローバル `require`
スタブに落ち、6章までと同じ挙動（明確なエラー）になる -- 直したのは
「自分自身の内部ファイル分割」だけで、真の外部依存解決は7章同様
引き続きスコープ外のまま。

**検証**: `require` 検出・パス正規化・バンドル本体それぞれに
オフラインのユニットテストを追加した上で、生成されたバンドルを
実際に thaw-quickjs（QuickJS-NG）に通して実行するテストも追加し、
相対 require で分割されたファイル間の呼び出しが正しく動くことを
実機で確認した。さらに実際の `qs` パッケージで `thaw registry add qs`
を実行し、5ファイルが正しく1つの `bundle.js` にバンドルされることを
確認した（`qs` の `.d.ts` 自体は関数を `declare namespace` の中に
書いており、現状の thaw-bridge の抽出ロジックは名前空間の中を見ない
ため、実際に `--use qs` から関数を呼び出すところまでは別課題として
残っている -- 9章参照）。単発ファイルのパッケージ（`left-pad` など）
がこの変更で壊れていないことも再確認済み。

## 9. `declare namespace` の中の関数宣言（対応済み）

8章の `qs` 検証で見つかった課題を対応した。詳細は
[bridge.md 3.1節](bridge.md)参照 -- thaw-bridge の `parse_dts` が
`declare namespace Foo { function bar(...): ...; }` という形（`qs`
自身がまさにこの形）で名前空間の中に書かれた関数も再帰的に抽出する
ようになった。抽出した名前は namespace 修飾なしの裸の名前（`parse`
など）で、これは6章の CommonJS ラップ機構が既にサポートしている
「`module.exports` がオブジェクトならプロパティごとにグローバルへ
hoist する」という挙動と自然に噛み合う。

**検証**: 手作りの namespace 付きパッケージ（依存なし）を
`--use` 経由でビルド・実行し、`double(21)` → `42`、
`greet("thaw")` → `"hi, thaw"` を確認した。実際の `qs` でも
`thaw registry add qs` → `thaw build --use qs` で関数の生成
（`parse`/`stringify` の Fallback wrapper）自体は正しく行われることを
確認したが、`qs` は `side-channel`/`es-define-property` という
**真の外部パッケージ依存**を持っており（`package.json` の
`dependencies`）、これは6/10章で明示的にスコープ外としている
依存解決の問題であって、今回直した名前空間抽出とは別の課題。
`qs` を完全に動かすには、次の10章にある「真の外部依存解決」が
別途必要になる。

## 10. 真の外部パッケージ依存の解決（対応済み）

9章末尾で見つかった `qs` の実依存（`side-channel`/`es-define-property`）
を解決できるところまで実装した。鍵となる観察: `npm install qs` を
1回実行するだけで、npm 自身が `qs` の依存関係グラフ全体（直接・間接
問わず）を解決し、`node_modules/` 直下に（現代の npm のフラット化
戦略により、ほぼ全て）展開してくれる。つまり「レジストリが依存解決を
やる」という重い課題は、実は「npm が既にやってくれた結果を、
スクラッチディレクトリが消される前に見つけ出すだけ」という軽い課題に
還元できた -- 2回目のネットワーク往復も不要。

`bundle_commonjs_package` を拡張した:

- モジュールのキーを `"<パッケージ名>/<パッケージ内の相対パス>"`
  という形（パッケージ名で修飾）に変更 -- 複数パッケージをまたいで
  バンドルするようになったため、`index.js` のような衝突しやすい
  ファイル名でも別パッケージ同士で干渉しない。
- 8章の相対 require 解決に加え、**裸のパッケージ名の require**
  （`require('side-channel')` など）も見つけたら、
  `node_modules_dir/<名前>/package.json` を読んで `main` を解決し、
  そのパッケージも同じ要領で（そのパッケージ自身が持つさらなる
  requireも含めて）再帰的にバンドルに取り込む。ループもキューも
  8章のものをそのまま流用 -- パッケージ境界をまたぐかどうかを
  意識しないコードになっている。
- **deep import**（`require('es-errors/type')` のような、パッケージ名の
  後に部分パスが続く形）にも対応した。`qs` の実依存チェーンを実際に
  辿ったところ、これが本当に必要になった（`side-channel` の依存の
  1つがこの形で require していた）。`split_bare_spec` がパッケージ名と
  部分パスを分離し、部分パスがあればそのパッケージの `main` を無視して
  部分パスをそのままそのパッケージのルートに対して解決する（Node の
  実際の挙動と同じ）。
- `node_modules_dir` に存在しない裸の require（Node のコアビルトイン
  `fs`/`util`など、または本当にインストールされていない依存）は
  8章までと同様、解決せずそのまま残す -- 実行時に外部 require
  スタブが明確なエラーを出す。

**もう1つ見つけて直した問題**: 複数の `--use` パッケージは全て
「同じ」スレッドローカルな QuickJS-NG グローバルコンテキストに
`loadScript` される（パッケージごとに1回、6章の
`wrap_as_commonjs_module`）。バンドル内部の状態
（`__thaw_bundle_cache` 等）を素のグローバル変数のまま実装していると、
2つ目のパッケージを読み込んだ瞬間に1つ目のパッケージの内部状態を
**上書き**してしまう。モジュール読み込み時（トップレベル）に即座に
解決される require なら影響が見えない（読み込み完了時点で結果は
確定済みのため）が、**関数呼び出し時まで遅延される内部 require**
（関数本体の中で呼ばれて初めて実行される require）は、2つ目以降の
パッケージが読み込まれた**後に**呼ばれると、誤って別パッケージの
モジュールマップを参照してしまう恐れがあった。対策として
`render_bundle` の出力全体を IIFE で包み、`module.exports = ` の
代入結果だけを外に出すようにした -- バンドル内部のヘルパーは
一切グローバルに漏れず、クロージャが自分自身の（他のどのパッケージの
読み込みにも影響されない）プライベートな状態を保持し続ける。

**検証**: `bundle_commonjs_package`/`split_bare_spec` に新規オフライン
ユニットテストを追加した上で、実際に thaw-quickjs（QuickJS-NG）に
通して: (1) 相対 require と裸の require（`node_modules` 内の実在
パッケージ）が両方とも同じバンドルの中で正しく解決されること、
(2) 2つの独立したパッケージのバンドルを同じグローバルコンテキストに
連続で読み込んだ後でも、1つ目のパッケージの遅延内部 require が
正しく（2つ目のパッケージの状態に汚染されずに）解決されること、を
実機で確認した。実際の `qs` では `thaw registry add qs` が46ファイル
（`qs` 本体＋`side-channel`＋その先の依存チェーン全体）を正しく
1つの `bundle.js` にバンドルし、実際に `thaw build --use qs` で
`parse`/`stringify` を呼び出せることを確認した -- ただし `qs` の
依存チェーンのどこかが最終的に Node のコアビルトイン `util` を
require しており、QuickJS-NG にはそもそも `util` モジュールが
存在しないため、そこで明確なエラーになって止まる（11章）。
また、`is-odd`（3章で「真の外部依存を持つため非対応」とした最初の
実例）が、依存先の `is-number`込みで正しくバンドルされ、
`isOdd(3)` → `true`、`isOdd(4)` → `false` を実際に返すことも確認した
-- 3章で見つけた課題が、今回でようやく完全に解決された。
`left-pad`/`ms` など既存パッケージが壊れていないことも再確認済み。

## 11. Node のコアビルトインモジュール（最小限の polyfill で対応開始）

`qs` の依存チェーンを実際に最後まで辿ったことで見つかった、10章とは
性質の異なる課題: `require('util')` のような **Node.js 本体が提供する
コアビルトインモジュール**への require。これは npm パッケージでは
ないので `node_modules` の下には存在せず、10章の仕組みでは
（正しく）解決されない。QuickJS-NG は Node.js ランタイムではないため
`util`/`fs`/`path` などのモジュールはそもそも実装が存在しない。

「Node.js 標準ライブラリを再実装する」というのは際限なく広がりうる
作業なので、**実際に必要になった分だけ**その場で追加する、という
このドキュメント全体を貫く方針をここでも踏襲した。`builtin_module_source`
という小さな対応表を追加し、10章のバンドラが裸の require を
`node_modules` の下で見つけられなかったときの最後のフォールバックと
した。実装したのは `util` のみ、しかも `util` 全体ではなく:

```js
function inspect(value) { return String(value); }
inspect.custom = Symbol.for('nodejs.util.inspect.custom');
module.exports = { inspect: inspect };
```

これだけ。理由: `qs` を止めていたのは `object-inspect`（`side-channel`
経由の間接依存）の `util.inspect.js` が `require('util').inspect` を
**無条件にトップレベルで**行っていたことだが、`object-inspect` 自身が
独自の値整形ロジックを実装しており、本物の `util.inspect` は
`.custom`（Node がカスタム整形関数を認識するための `Symbol`）を
取り出すためだけに使われていた。つまり本物の `util.inspect` の
出力仕様（循環参照検出、色付け、深さ制限など）を再現する必要は
一切なく、「関数として呼べて `.custom` という Symbol プロパティを
持つ」という最小限の形さえ満たせば十分だった。

**検証**: 実際に `object-inspect` と同じパターン（`util.inspect.js`
のコードそのまま）で新規テストを追加し、`.inspect.custom` が実際に
`Symbol` 型として返ってくることを thaw-quickjs 経由で実行確認した
上で、実際の `qs` を最初から通しで検証: `thaw registry add qs` →
`thaw build --use qs` で、**手作業を一切介さず**
`parse("a=1&b=2")` → `{"a":"1","b":"2"}`、
`stringify({"a":"1","b":"2"})` → `"a=1&b=2"` という正しい結果を得た。
1章で「レジストリの自動化が何もない」ところから始まり、6〜11章の
積み重ねで、実在する・そこそこ複雑な npm パッケージが1つ、完全に
「npm と同じ感覚」で使えるようになった、最初の実例。

`util` 以外のコアビルトイン（`fs`/`path`/`stream` など）は、実際に
それらを要求する別のパッケージにぶつかった時点で、同じ要領
（`builtin_module_source` に1エントリ追加する）で対応していく。

追記: 14章の ESM 対応の一環で `process` も同じ要領で追加した。
実際の ESM パッケージ（`has-flag`）が `import process from 'process'`
と、モジュールとして import していたため（Node は `process` を
グローバルとコアモジュールの両方として提供する）。`argv`/`env`/
`platform`/`nextTick` など、実際に読まれたフィールドだけを持つ
最小限のオブジェクトで、`.default` も同じオブジェクトを指すように
しておくことで、ESM の default import 相互運用規約とも噛み合う。
`thaw registry add has-flag` → `thaw build --use has-flag` で
実際に `hasFlag('unicorn', ['--unicorn', '--foo=bar'])` → `true` を
確認した。

## 12. スコープ付きパッケージの実地検証、プラットフォームグローバルの対応

`@scope/name` 形式のスコープ付きパッケージは、これまで
`types_package_name`/`split_bare_spec` 等のユニットテストでしか
確認しておらず、実際に `npm install` して最後まで通した検証が
なかった。`@hapi/hoek`（型定義バンドル済み・依存なしの実パッケージ）
で試したところ、`thaw registry add @hapi/hoek` 自体は問題なく成功した
（`node_modules/@scope/name/` という実ディレクトリ構造は元々
`Path::join` がそのまま扱えていたため）。ただし実行時に、11章までの
「npm パッケージの require グラフ」とも「Node コアビルトインモジュール」
とも異なる、3つ目の種類の互換性の穴が2つ見つかった:

- **`Buffer && Buffer.isBuffer(x)`**: `@hapi/hoek` のコードは
  Node 専用グローバルを使う前にちゃんと truthy チェックで**ガード**
  していた。しかし JS では、一度も宣言されていない裸の識別子への
  参照はガードの中であっても（真偽値評価のために識別子解決が必要な
  ため）`ReferenceError` を投げる -- ガードの意図（「Buffer が
  使えない環境では諦める」）を活かすには、`Buffer` という名前
  **自体**が存在しさえすればよい。`wrap_as_commonjs_module` に
  `if (typeof globalThis.Buffer === 'undefined') { globalThis.Buffer
  = undefined; }` を追加するだけで解決した -- 実際に Buffer を
  実装する必要は一切ない。
- **`URL.prototype`**（ガードなし）: 同じパッケージの別の場所で、
  今度はガードせずに `URL.prototype` を直接参照していた。`undefined`
  では `.prototype` アクセスに耐えられないため、こちらは空の
  コンストラクタ関数 `function URL() {}` を代わりに用意した
  （JS の関数は自動的に `.prototype` オブジェクトを持つため、
  この最小限のスタブで十分だった）。

どちらも「本物の実装を用意する」のではなく「参照してもエラーに
ならない最小限の見せかけを用意する」という、11章の `util` polyfill
と同じ考え方 -- 実際に必要になった分だけ、その場で対応する。

**検証**: 新規ユニットテストで、ガード済み `Buffer` 参照とガードなし
`URL.prototype` 参照の両方が実際に thaw-quickjs 上で例外を投げずに
実行できることを確認した上で、実際の `@hapi/hoek` を通しで検証:
`thaw registry add @hapi/hoek` → `thaw build --use @hapi/hoek` で
`escapeRegex("a.b*c")` → `"a\.b\*c"`、
`escapeHtml("<b>hi</b>")` → `"&lt;b&gt;hi&lt;&#x2f;b&gt;"` という
正しい結果を得た -- スコープ付きパッケージの初めての完全な実地検証。

## 13. 複数パッケージ間の名前衝突を検出してエラーにする

ここまで検証してきた5パッケージ（`left-pad`/`ms`/`is-odd`/`qs`/
`@hapi/hoek`）を1つのプログラムで同時に `--use` する、初めての
「複数の実パッケージを組み合わせて使う」検証で見つかった問題:
`qs` も `@hapi/hoek` も、どちらも `stringify` という関数をエクスポート
している。両方を `--use` すると、後から読み込まれた方の `stringify`
が黙ってグローバルスコープを上書きし、`--use` の指定順序だけで
どちらが呼ばれるかが決まってしまっていた（実際に順序を入れ替えて
再現・確認した）。

これは Fast path / Fallback を問わず、生成される全てのトップレベル名が
（QuickJS-NG のグローバルスコープにせよ、LLVM モジュールのシンボル
テーブルにせよ）1つのフラットな名前空間を共有しているために起きる。
4.3節で見つけた「同じ `.d.ts` 内でのオーバーロード衝突」
（`classify_all` で対応済み）と全く同じ性質の問題が、今度は
「別々の `.d.ts`（別パッケージ）間」で起きた形。

対応として、`thaw-cli` の `generate_registry_shims` に、生成される
トップレベル名ごとに「どの `--use` パッケージが最初に宣言したか」を
記録し、**別のパッケージが同じ名前を宣言しようとしたらビルドを
明確なエラーで止める**チェックを追加した。名前を自動でリネームしたり
namespace 修飾したりする機能（`qs.stringify` のような書き方）は
まだ存在しないため、衝突した場合はユーザーが手動でどちらかの
`--use` を諦めるしかないが、少なくとも「順序に依存してどちらかが
黙って勝つ」という、気づきようのない誤動作は防げる。

**検証**: `left-pad`/`ms`/`is-odd`/`qs`/`@hapi/hoek` を全て
`--use` すると、`` `stringify` is declared by both `qs` and
`@hapi/hoek` `` という明確なエラーで即座に止まることを確認した。
衝突のない4パッケージの組み合わせ（`qs` を除いた分）は問題なく
ビルド・実行できることも確認済み。

## 14. ESM 専用パッケージへの対応

`import`/`export` 構文をそのまま使う ESM 専用パッケージ
（`package.json` の `"type": "module"`）は、これまで完全にスコープ外
だった -- QuickJS-NG のスクリプト評価モード（`ctx.eval`）は
`import`/`export` 構文をそのまま実行できないため、6章までの
CommonJS/UMD 対応では歯が立たなかった。

対応方針は、実行時に本物の ES Module 評価モードを使うのではなく、
**バンドル時（`add` の時点）に ESM 構文を CommonJS 相当のコードへ
書き換えてしまう**というもの -- そうすれば8〜13章までに作った
require グラフ解決の仕組み（相対 require、外部パッケージ、deep
import、ビルトイン polyfill）をそのまま再利用できる。

- `thaw-parser` に `parse_javascript_with_source_map` を追加した。
  `parse_javascript`（`parse_module()` を使う、ES Module 構文を
  ネイティブに解釈できる既存関数）の結果に加え `SourceMap` も返す --
  ノードの元のソーステキストを `SourceMap::span_to_snippet` で
  正確に取り出すために必要（`Span` の生のバイトオフセットを自分で
  計算するのは安全ではないため）。
- `thaw-registry` の `rewrite_esm_to_commonjs` が実際の書き換えを行う:
  ファイルをパースして `ModuleDecl`（`import`/`export`）が1つでも
  あれば ESM と判定し、`import`/`export` 宣言だけを合成した
  `require(...)`/`exports.x = ...` に置き換え、それ以外の文は
  **AST から再生成せず、元のソーステキストをそのまま**
  （`span_to_snippet` で）コピーする -- このプロジェクトは汎用の
  JS コード生成器を持たないため、触らない部分は一切再フォーマットせず
  温存するという方針。ESM 構文が1つも見つからなければ `None` を返し、
  呼び出し側は元のソースをそのまま使う -- 既存の CommonJS パッケージを
  壊すリスクは原理的にない。
  - `import x from './y'` → `require('./y')` の呼び出し結果から
    `.default`（`.__esModule` フラグで本物の ESM 由来かどうかを
    判定し、そうでなければオブジェクト全体を default 扱いする、
    Babel 等が使うのと同じ相互運用規約）を取り出す。
  - `import { a, b as c } from './y'`/`import * as ns from './y'`/
    `export { a } from './y'`/`export * from './y'` もそれぞれ対応。
  - `export default <値>`/`export function foo(){}`/`export const
    x = ...` は、対応する CommonJS の書き方（`module.exports.default
    = ...`/`exports.foo = foo;` など）に変換する。
  - この書き換えは require グラフを辿る**全てのファイル**に対して
    行われる（`main` だけでなく、内部の相対ファイルや外部依存も含む）
    ので、パッケージの一部だけが ESM、というケースも自然にカバーする。
  - `resolve_module_path` の解決候補に `.mjs` を追加した（実際の ESM
    パッケージが `.js` の代わりに使うことが多いため）。
- **もう1つ見つけて直した統合上のバグ**: `escape-string-regexp`
  （実際の ESM パッケージ）で検証したところ、`export default function
  escapeStringRegexp(){}` は書き換え後 `module.exports.default = ...`
  になるが、6章の `wrap_as_commonjs_module` の default-export 束縛
  ロジックは `typeof module.exports === 'function'` しか見ておらず、
  `module.exports` 自体がオブジェクト（`{ __esModule: true, default:
  fn }`）のこのケースを見逃していた。`module.exports.default` が
  関数かどうかも追加でチェックするよう修正した。
- **もう1つ見つけて直した `.d.ts` 抽出の穴**: `escape-string-regexp`
  自身の `.d.ts` も `export default function
  escapeStringRegexp(string: string): string;` という書き方をしており、
  これは `export declare function foo(){}`（`ExportDecl`）とは別の
  AST ノード（`ExportDefaultDecl`）のため、thaw-bridge の `parse_dts`
  は当初これを抽出できなかった。名前付きの `export default function`
  も抽出するよう `extract_fn_decls` を拡張した（無名の場合は
  呼び出し可能な名前が存在しないため、諦めて0関数のまま -- クラッシュ
  はしない）。

**検証**: 書き換えロジック自体のユニットテストに加え、ESM の
`import`/`export` で分割された2ファイルのバンドルを実際に
thaw-quickjs（QuickJS-NG）で実行するテスト、`module.exports.default`
の束縛修正についても同様に実機で確認した。実際の ESM 専用パッケージ
`escape-string-regexp`（`"type": "module"`、単一の default export
関数）で通しの検証を行い、`thaw registry add escape-string-regexp` →
`thaw build --use escape-string-regexp` で `escapeStringRegexp("a.b*c")`
→ `"a\.b\*c"` という正しい結果を、手作業を一切介さず得た。

## 15. 名前衝突の自動解決: namespace 修飾構文

13章では「検出してエラーで止める」ところまでだった。ユーザーが
`qs.stringify(x)`/`hoek.stringify(x)` のように**書けば**衝突を
自分で解決できるようにした -- ただし Thaw の言語機能として
本物のオブジェクト・メンバー呼び出しを実装したわけではない。
`qs.stringify(x)` は**コンパイラに一切触れず**、`thaw-cli` の
前処理段階だけで `qs_stringify(x)` という、衝突しない実在の
関数呼び出しに書き換えてしまう、純粋なソースレベルの糖衣構文
として実現した -- パーサー/HIR への投資を避けるという、この
プロジェクト全体の方針を踏襲している。

1. `thaw-cli` はまず、`--use` された全パッケージの分類結果を集め
   （`generate_registry_shims`）、同じ名前を2つ以上のパッケージが
   宣言していれば衝突として記録する。片方でも Fast path
   （ネイティブシンボルの衝突）なら、これは自動解決できない実体の
   衝突なので13章通りエラーのまま。全て Fallback なら次に進む。
2. **衝突の有無に関わらず**、`--use` した各パッケージの**全ての**
   Fallback 関数に対して、パッケージ修飾したエイリアス識別子
   （`qs_stringify`）と、実行時の JS グローバル参照キー
   （`"qs::stringify"`、`::` 区切りなのでバケット記法のオブジェクト
   キーとしてのみ使い、衝突しない）を生成する
   （`thaw_bridge::QualifiedFallback`）。**衝突していない名前**は
   素の名前（`stringify(x)`）とパッケージ修飾形（`qs.stringify(x)`）
   の**両方**で呼べる。**衝突した名前**は素の名前を生成せず、
   パッケージ修飾形でしか呼べなくする
   （`QualifiedFallback::suppress_bare`）-- 呼べてしまうと結局
   「後から読み込んだ方が勝つ」という13章の問題に逆戻りするため。
3. `thaw-bridge` の `generate_module_init` は、各パッケージの
   `loadScript` 呼び出しの**直後**（次のパッケージが読み込まれて
   素の名前を上書きする**前**）に、素の名前を修飾キーの下にも
   コピーする追加の `loadScript` 呼び出しを生成する
   （`ModuleBundle::qualified_aliases`）。**これも実行時の JS**
   として動く必要があるため、`__thaw_module_init`（普通にコンパイル
   される Thaw 関数）の中に生の `typeof`/角括弧アクセスを直接書く、
   という実装ミスを最初にやってしまい、実機で検証して気づいた
   （Thaw のコンパイラは `typeof`/`!==` を理解しないので
   即座にコンパイルエラーになった）。正しくは、その JS 片も
   `loadScript("...")` の文字列引数として、二重にエスケープして
   埋め込む必要がある。
4. スコープ付きパッケージ名（`@hapi/hoek`）はそのままでは有効な
   JS/Thaw 識別子になれない（`@`/`/` を含む）ため、ユーザーが
   `pkg.name(...)` の `pkg` 部分に書く「修飾子識別子」には
   パッケージ名の最後のパスセグメントを使う（`@hapi/hoek` →
   `hoek`）。これも実際に `hoek.stringify(...)` と書いて動かない
   ことに気づいて直した（最初は `@hapi/hoek.stringify(...)` という
   構文的に無効なコードを想定していた）。2つの異なるスコープ付き
   パッケージが同じ最終セグメントを持つケース（`@foo/utils` と
   `@bar/utils`）は未対応（今のところ実例に遭遇していない）。
5. `thaw-cli` は最後に、ユーザー自身の `.ts` ソースを（`swc_ecma_visit`
   の `Visit` でファイル全体を走査し）解析して `pkg.name(...)` という
   形の呼び出し式を全て見つけ、対応するエイリアス識別子に**該当
   スパンだけ**書き換える（`rewrite_qualified_calls`）-- 該当しない
   コードは一切触らない。thaw-registry の ESM 書き換え（14章）と
   同じ「触らない部分は元のソーステキストをそのままコピーする」方針
   だが、今度は**ファイル全体を再帰的に**辿る必要がある（呼び出しは
   式の中のどこにでもネストしうるため）ので、8章までの「トップレベル
   の項目だけを見る」書き換えよりも一段複雑になっている。

**検証**: `rewrite_qualified_calls`/`qualifier_identifier`/
`sanitize_identifier` のオフラインユニットテストに加え、実際の5
パッケージ（`left-pad`/`ms`/`is-odd`/`qs`/`@hapi/hoek`）を全て
`--use` し、`qs.stringify(...)`/`hoek.stringify(...)`
（衝突していた名前）と `qs.parse(...)`/`hoek.escapeHtml(...)`
（衝突していない名前）を全て namespace 修飾構文で呼び出して、
それぞれ正しい結果を得た。衝突していない名前は素の呼び出し
（`parse(...)`）でも同じ結果になることも確認済み -- 13章で
見つかった問題が、ユーザーが書けるコードの形で最終的に解決された。

## 16. ネイティブアドオンの失敗を、意味の分かる失敗にする

「ネイティブアドオンへの対応」を素直に取ると「`.node` バイナリを
QuickJS-NG から呼び出せるようにする」になるが、それは N-API/V8 の
呼び出し規約を丸ごとホストするに等しく、8章で明示的にスコープ外と
決めた「ネイティブライブラリのビルドパイプライン」よりさらに重い。
今回はそこまでは踏み込まず、**実際に2つの実物パッケージで何が
起きるかを観察してから**、現実的に価値のある一手だけを取った。

- `utf-8-validate`（`node-gyp-build` に依存、ネイティブ読み込みに
  失敗したら `require('./fallback')` という純粋 JS 実装に
  フォールバックする設計）: 6章の `require` スタブが
  `require('node-gyp-build')` の時点で例外を投げる → パッケージ自身の
  `try/catch` がそれを正しく捕まえてフォールバック実装を使う、という
  形で**何も変更せずに最初から正しく動いていた**。
- `bcrypt`（同じく `node-gyp-build` に依存するが、`try/catch` なしで
  無条件に呼び出す設計）: `bindings = require('node-gyp-build')(...)`
  より前に、`bcrypt.js` 自身が書く `require('path')` で早々に
  スタブへ落ち、`require('path') is not supported in the Fallback
  path yet` という、ユーザーから見て原因の分からないエラーになった。

後者を実際に追いかけると、`node-gyp-build` の実体
（`node-gyp-build.js`、これも無改造のまま `bundle.js` に含まれている）
は `fs`/`path`/`os` を使ってプリビルド済みバイナリを探し、
見つからなければ**それ自身が**
`throw new Error('No native build was found for ' + target + ...)`
という、原因が一目で分かるエラーを投げるようになっている。つまり
「ネイティブアドオンの検出」を Thaw 側で新たに作り込まなくても、
`fs`/`path`/`os` の最小限の polyfill さえあれば、**実物の npm
エコシステムのコード自身に正しい診断を最後まで出させられる**。

- `path`: `resolve`/`join`/`dirname`/`basename` を文字列操作だけで
  実装（Node の `path` も実ファイルシステムには触れないので、これで
  十分）。
- `os`: `arch()`/`platform()`/`tmpdir()`/`EOL` の固定値。
- npm bundle内部の`fs` polyfillは、`existsSync`が常に`false`、
  `readdirSync`/`statSync`/`readFileSync`が`ENOENT`相当を投げる保守的なshimの
  ままである。一方、ユーザーの`node:fs` importは`thaw-std` Fast Pathへ接続し、
  UTF-8 `readFileSync`/`writeFileSync`、`existsSync`、recursive `mkdirSync`を実際の
  filesystemに対して実行する。このレジストリ
  は `package.d.ts`/`bundle.js` しか保存しないため（7章）、
  「実ファイルは何も無い」が文字通り正しい答えであり、
  `node-gyp-build.js` 自身がその前提で `try/catch` して `[]` を
  返す設計になっている。
- `process` を**グローバルとしても**定義（11章では `require('process')`
  用のモジュールとしてのみ存在していた）: `node-gyp-build.js` は
  `require` せずに `process.config`/`process.versions`/`process.env`/
  `process.execPath` をいきなり参照するため、Buffer/URL と同じ理由で
  未定義グローバル参照が `ReferenceError` になっていた。
- `__dirname`/`__filename` もグローバルとして定義（`fs` が常に
  「何も無い」と答える以上、実際の値は結果に影響しない -- 存在する
  ことだけが必要）。

**検証**: `bcrypt` を無改造のまま `--use` して `genSaltSync` を
呼び出すと、修正前は `require('path') is not supported in the
Fallback path yet` という無関係な一次エラーで落ちていたのが、修正後は
`node-gyp-build` 自身が書いた
`No native build was found for platform=linux arch=x64 runtime=node
...` という、原因が一目で分かるメッセージに変わることを確認した。
`utf-8-validate` 側は同じ変更後も引き続き `isValidUTF8("hello")` →
`true` と正しく動くことを確認済み（`fs`/`path`/`os`/グローバル
`process` の追加が、既に動いていた JS フォールバック経路を壊して
いないことの回帰確認）。`path`/`os`/`fs` polyfill 自体と、
グローバル `process`/`__dirname`/`__filename` が実際に QuickJS-NG 上で
動くことは、`node-gyp-build.js` の実際の探索ロジック（`try/catch` で
`fs.readdirSync` の失敗を吸収する部分を含む）を模したオフライン
ユニットテストでも確認している。

**まだ埋まっていない穴**: これは「失敗の質を上げた」だけであり、
「ネイティブアドオンを実行できるようにした」わけではない --
`.node` バイナリを実際にロードして呼び出すことは、依然として明確に
スコープ外（本章冒頭の理由の通り）。

## 17. バージョン指定と、実際に解決されたバージョンの記録

これまで `thaw registry add <package>` はパッケージ名だけを受け取り、
`npm install <package>`（常に最新版）を呼ぶだけだった。`npm install`
自体は `<package>@<version>` 形式のバージョン/タグ/範囲指定を昔から
受け付けるのに、`add` はそれをそのまま素通りさせず、パッケージ名を
そのままレジストリのディレクトリ名にも `npm install` への引数にも
使っていた -- スコープ付きパッケージでない限りは動いてしまうものの、
`left-pad@1.3.0` を渡すと `node_modules/left-pad@1.3.0` という
存在しないディレクトリを探しにいって壊れる、という具体的なバグが
あった（試して初めて気づいた類のもの、というより実装を読んで
気づいた）。

- `split_package_spec`（`thaw-registry`、`pub fn package_name` として
  裸のパッケージ名だけを外部にも公開）: `<package>[@<version-or-range
  -or-tag>]` を裸のパッケージ名とバージョン指定に分割する。スコープ
  付きパッケージの先頭 `@`（`@hapi/hoek`）をバージョン区切りと
  誤認しないよう、スコープの `/` より後ろにある `@` だけを見る。
- `npm install` にはユーザーが書いた文字列をそのまま渡す（`npm`
  自身のバージョン解決をそのまま使う -- 独自の semver 実装は書かない）。
  レジストリのディレクトリ名・`node_modules` 内の参照・
  `@types/*` フォールバックの対象名には、分割で得た**裸の名前**だけを
  使う（`@types/*` はバージョンが本体と独立に管理されているため、
  本体側のバージョン指定を引き継ぐ意味がない）。
- `npm install` が実際に解決したバージョン（フェッチ済みパッケージ
  自身の `package.json` の `version` フィールド）を
  `registry_dir/<package>/version.txt` として書き込む。バージョン
  指定なしで `add` した場合も、`npm` が「最新」として何を選んだかが
  そのまま記録される -- 「常に最新版」が見えない前提のままではなく、
  結果を確認できるようになった。`ResolvedPackage::version` として
  `resolve`（`--use` の解決経路）からも読めるようにしたが、ビルド時に
  自動で表示することはしていない（8章までの他のメタ情報と同じ扱い）。
- これはあくまで**パッケージ1つぶんの独立したバージョン解決**であり、
  `package-lock.json` 相当の依存グラフ全体のバージョン整合
  （このパッケージが要求する依存パッケージのバージョン範囲、など）は
  引き続き見ていない -- `npm install <spec>` を1回呼ぶのと同じ粒度。

**検証**: `left-pad@1.1.0` と `@hapi/hoek@9.0.0`
（スコープ付き+バージョン指定の組み合わせ）を実際に `add` し、
どちらも正しいディレクトリ名（`left-pad/`、`@hapi/hoek/`）に
`version.txt`（それぞれ `1.1.0`/`9.0.0`）が書き込まれることを確認、
`left-pad@1.1.0` を `--use` して実際にビルド・実行し正しい出力
（`00007`）を得た。バージョン指定なしの `add is-odd` も引き続き
動作し、`version.txt` に `npm` が選んだ実際のバージョン（`3.0.1`）が
記録されることを確認 -- 既存の無指定パスの回帰確認も兼ねる。
`split_package_spec` 自体はオフラインユニットテストで
スコープ付き/なし × バージョン指定あり/なしの4パターンを確認済み。

## 18. Fast path の Marshal アダプタ: `(ptr, len)` 展開とオブジェクトのフィールド展開

bridge.md 5章がスコープ外として保留していた項目: Fast path の FFI 呼び出しは
これまで、外部ネイティブ関数の ABI が **Thaw 自身の内部表現とそのまま
一致する**ことを前提にしていた（`number[]` は Thaw の
`[i64 len][f64 elements...]` バッファへの生ポインタ1個をそのまま渡す、
という具合）。これは「Thaw で書かれた別のネイティブライブラリを呼ぶ」
場合にしか正しく動かず、実在する C ライブラリの慣習（配列は
`(ptr, len)` の2引数に分けて渡す、構造体はフィールドごとの別引数として
渡すことが多い）とは一致しない。

- `number[]` パラメータ: 呼び出し直前に、Thaw のヘッダから長さを読み出し
  `(const double* elements, int64_t len)` という2引数に展開する
  （`thaw-llvm::hir_codegen::ffi_param_types`/`compile_ffi_call`）。
  シンボルの宣言側（`ffi_param_types`）と呼び出し側
  （`compile_ffi_call`）の両方を同じ形に対応させる必要があり、
  どちらか片方だけ直すとリンクエラーか未定義動作になる。
- object パラメータ（`{ x: number; y: number }` のような、Fast path
  分類が受け付ける number フィールドのみの形）: 単一の構造体ポインタ
  ではなく、`.d.ts` 宣言順のフィールドをそれぞれ個別の C 引数として渡す
  （`distance(x1, y1, x2, y2)` のような、実際の C API によくある形）。
  実際のターゲットの構造体渡し ABI（レジスタ分類規則、パディングなど）を
  汎用的に再現するのは別の課題として見送り、フィールド単位への展開の
  方を採用した。
- 戻り値側のマーシャリング（`Array`/`Object` を返す C 関数への対応）と、
  文字列を `(ptr, len)` に分割する規約への対応は、依然としてスコープ外
  のまま（`string` はそのまま `const char*` として渡す、これまでの規約
  のみ対応）。

**検証**: 実際に手書きの（Thaw の内部レイアウトを意識せず書いた）C
関数と実際にリンクして確認した。`number[]` 側は
`double native_sum(const double* xs, int64_t len)`（配列の中身と長さの
両方を実際に使う実装）に `[1,2,3,4]` を渡して `10` を得た。object 側は
`double native_combine(double x, double y) { return x*10+y; }` に
`{x:3, y:4}` を渡して `34`（フィールド順が入れ替わっていれば `43`に
なるはずの値）を得て、フィールド順の対応も確認した。既存の Fast path
テスト（プリミティブ引数のみ）は無変更のまま全て通ることも確認 --
この変更は `Array`/`Object` パラメータの扱いだけを変え、それ以外の
呼び出し規約には影響しない。

## 19. 依存グラフ全体のバージョン記録: `lock.json`

17章で `add` 自身のバージョン指定・記録はできるようになったが、
記録していたのは `package` 自身のバージョンだけだった。実際の npm
パッケージ（`qs`）は `side-channel`/`object-inspect`/`get-intrinsic`
など多数の実パッケージに依存しており（10章で解決済みの、バンドルへの
実取り込み自体は元々できていた）、それらのバージョンは一切記録されて
いなかった -- `package-lock.json` 相当のものが「無いに等しい」以前に、
そもそも npm が実際に何を選んだのかを**確認する手段自体が無かった**。

- `bundle_commonjs_package` のバンドル走査（10章のワークリスト）は
  もともと同一パッケージ内の相対 `require` とパッケージ間の bare
  `require` の両方を辿っていた。この各ステップで新しく訪れた実
  パッケージ（ビルトイン polyfill は対象外）ごとに、その
  `package.json` から `version` を読み取って記録するようにした
  （`record_package_version`）-- 対象を1個増やしただけで、
  ネットワークアクセスや `npm` の再呼び出しは一切増えていない。
  1回の `npm install <spec>` が実際に取得したファイルを読むだけ。
- 集めた `{ パッケージ名: バージョン }` の全体を
  `registry_dir/<package>/lock.json` として書き込む（`package` 自身
  1件だけなら `version.txt` と内容が重複するだけなので書かない）。
  `AddedPackage::dependency_versions`/`ResolvedPackage::
  dependency_versions` としても公開し、`thaw registry add` の
  CLI 出力にも `deps:` 行として表示するようにした。
- これは依然として**記録**であって**解決**ではない: 独自の semver
  範囲ソルバは書いていない（`npm install` 自身の解決結果をそのまま
  読むだけ）。複数の `add` 呼び出しをまたいだバージョン整合
  （パッケージ A・B が異なるバージョンの C に依存する場合の
  一本化、など）は引き続き見ていない。

**検証**: 実際に `qs` を `add` し、CLI 出力の `deps:` 行と
`lock.json` の両方に、実際に npm が解決した18個の実依存パッケージ
（`side-channel`/`object-inspect`/`get-intrinsic` など）とその
バージョンが正しく記録されることを確認した。依存を持たない
`is-odd` は `is-number` という実依存が正しく記録される一方
（`is-odd` 自体は `is-number` に依存するが、単一ファイルパッケージの
`solo-pkg` 相当のケースとは違い2件になるので `lock.json` が書かれる）、
記録すること自体はバンドル・実行の既存動作に一切影響しないことも
確認した -- `qs.stringify`/`isOdd` を実際に `--use is-odd --use qs`
でビルド・実行し、この変更の前後で出力が変わらないことを確認済み。
オフラインユニットテストでも、依存ありパッケージ（ルート+1件）と
依存なしパッケージ（ルートのみ1件）の両方で正しい版数が記録される
ことと、`resolve` が既存の `lock.json` を正しく読み戻せることを
確認している。

## 20. その他、今回やらなかったこと（意図的なスコープ外）

- **バージョングラフ全体の"解決"**: 19章の通り、実際に npm が選んだ
  バージョンを依存グラフ全体にわたって**記録**できるようになったが、
  独自の semver ソルバは書いていない。複数の `add` 呼び出しをまたいだ
  バージョン整合（パッケージ A・B が異なるバージョンの C に依存する
  場合の一本化など）はまだ見ていない。
- **ネイティブアドオンの実行そのもの**: 16章の通り、失敗時の診断は
  改善したが、`.node` バイナリを Thaw から実際に呼び出すパイプライン
  （N-API 相当のホスト実装）はまだない。設計は
  [native-addons.md](native-addons.md) にまとめてある（実装は未着手、
  実装すべき内容の具体的な書き下しのみ）。
- namespace 内で宣言された `interface`/`type`（9章末尾）。
- **その他のプラットフォームグローバル**: `TextEncoder`/`TextDecoder`、
  `setTimeout`/`clearTimeout`、`setInterval`/`clearInterval`、
  `queueMicrotask` までは QuickJS realm と Promise driver に実装した。
  網羅的な Node/Web API 対応表はまだ用意しておらず、これ以外は実際に
  参照するパッケージにぶつかった時点で追加していく（12章と同じ方針）。
- Marshal アダプタは、パラメータ側の配列／object展開、Array／Objectの
  portable struct戻り値、文字列の`(ptr, len)`引数／戻り値、配列戻り値の
  ownershipまで対応した。target固有struct packing、variadic、nested
  aggregate ownershipは引き続き対象外である。

## 21. ユーザーコードの bare import と package exports

`thaw build` は相対 TypeScript モジュールグラフを先に走査し、bare
specifier を見つけると同名のローカルregistry packageを自動解決する。
従来必要だった `--use` を明示しなくても、同じ `.d.ts` 分類、shim生成、
`bundle.js` 初期化、native archive／N-API選択が行われる。named、default、
namespace import はpackage固有のshim symbolへ変換されるため、同名exportを
持つ複数packageも同じプログラムから利用できる。

`registry add` のroot entry選択では、`package.json` の `exports["."]` を
読み、型は `types` condition、実行時は `require`、`import`、`default` の
順で選ぶ。該当conditionがなければ従来通りトップレベルの
`types`/`typings` と `main` にフォールバックする。`./feature` のような正確な
package subpath exportは、登録時に専用の型定義とruntime bundleを
`subpaths/feature/`へ保存し、`pkg/feature` importからrootとは独立して解決する。
`./features/*` のような単一wildcard exportも、型定義targetに一致する実在
ファイルを登録時に列挙し、同じ置換値をruntime targetへ適用する。すでに
依存を取得済みのvendor/offline workflowでは `add_installed` が同じ登録処理を
公開する。package export arrayは先頭から利用可能なtargetを選び、runtime／types
target内に`*`が複数現れる場合は同じcaptureをすべてへ適用する。export key自体に
複数の独立wildcardを持たせる形式は曖昧性があるため対象外である。

`node:path`、`node:util`、`node:process`、`node:buffer` は11章でnpm内部の
`require`向けに使ってきたpolyfillを、ユーザーのimportにも公開する。
現在の呼び出し規約はQuickJS fallbackと同じく、位置引数を格納した1つの
`Json` 配列である。

## 22. ネイティブNode組み込みモジュール

ユーザーコードからimportする `node:fs` はQuickJS polyfillではなく
`thaw-std` のネイティブFast Pathへ接続し、同期UTF-8ファイル操作を提供する。
同じ仕組みで `node:http` の最初の縦断実装として
`serveOnce(port: number, body: string): string` を追加した。この関数はloopbackで
1リクエストを受け、固定テキストのHTTP 200レスポンスを返し、request targetを
呼び出し元へ返す。TypeScript import、HIR/LLVM、静的リンク済みELF、実TCP通信を
通す統合テストで検証する。

関数型はHIRの `Function(params, return)` として保持し、arrow functionはLLVMの
内部関数へ変換する。関数値はarena上の `[code pointer, captures...]` という
closure環境を指し、間接呼び出しではその環境を隠し第1引数として渡す。これにより
外側の値を読むclosure、nested closure、Rustからのcallback呼び戻しが可能になった。
`serveOnceWith(port, (target) => body)` はこのABIを使い、callbackの戻り値を実際の
HTTP response bodyとして送信する。capture entryは値のコピーではなくarena上の
variable cellを指すため、closure生成後に外側で行った代入とclosure内の代入を
双方から観測できる。nested closureが作成元の呼び出しを抜けた後もcellは有効である。

objectのfunction型propertyは一般のclosureとして呼び出せる。これを利用した
`createServer(callback).listen(port)` は、callbackへ
`IncomingMessage { method, url }` と
`ServerResponse { statusCode, setHeader, write, end }` を渡す。status、header、分割write、
endの本文は実際のHTTP responseへ反映される。`listen` はsocketを登録して即座に返り、
生成されたC entry pointがTypeScriptのmain完了後に登録済みlistenerを駆動して、同じ
socketで逐次リクエストを継続処理する。テストやbatch用途では
`listenMany(port, count)` が従来どおり同期的に指定数を処理して返る。複数リクエスト間でも
callbackのclosure stateは保持される。`close()` は共有Atomic状態を通じて冪等に停止要求を
設定し、process lifecycle loopは閉じたlistenerを除去して、listenerがなくなると終了する。
listenerはthaw-runtimeの永続fd watcherとして登録され、timer、Promise continuation、
非同期HTTPと同じ `poll(2)` 呼び出しでreadable通知を受ける。server callbackの実行は
現在も単一threadの逐次実行だが、accept済みsocketも個別のread/write watcherとして
登録される。headerを送り切らない遅い接続やresponse backpressureが、他接続のaccept・
request処理を停止しない。`close()` は新規acceptを停止し、処理中の接続が完了してから
process lifecycle loopが終了する。`listen(port, callback)` のcallbackはlistener登録後の
event loop開始時に、`close(callback)` のcallbackはaccept済み接続がすべて完了した後に
単一thread上で呼ばれる。`server.on(event, callback)` は `listening`、`close`、`error` の
listenerを複数、登録順に保持し、再listen後も残す。listen/closeへ直接渡したcallbackは
一回だけ発火する。`on("error", callback)` は引数付きの内部ABIへloweringされ、`message`、
`code`、`syscall`、`address`、`port`を持つ構造化objectを登録順に通知する。不正portは
`ERR_SOCKET_BAD_PORT`、bind競合は`EADDRINUSE`となる。listenerのないerrorをprocess failureへ
変換する処理は次段階である。

## 23. platform optional dependencyのN-API prebuild

`@parcel/watcher`のように、本体packageの`prebuilds/`ではなく
`@parcel/watcher-linux-x64-glibc`等へbinaryを分離するpackageに対応した。
`optionalDependencies`から現在のplatform／architecture／libc suffixに一致するpackageを
選び、そのpackageの`main`が`.node`ならregistryの`native.node`へコピーする。source package
pathとhashは従来どおり`native-addon.json`へ記録する。

実npm integration testは`@parcel/watcher@2.5.1`を無改造で取得し、optional dependencyの
公式prebuildを埋め込んだ単一実行ファイルを生成する。registry directoryを削除した後に
Promiseベースの`writeSnapshot`を呼び、実snapshotファイルが作られるところまで検証する。

## 24. 実npm Promise workload

opt-in npm integration testは`p-limit@2.3.0`を無改造で取得し、依存する
`p-try@2.2.0`をregistry bundlerが自動解決したことをlock graphで確認する。生成された
CommonJS bundleをQuickJS fallbackへロードし、concurrency limiterを通したasync taskを
実行する。返却Promiseのmicrotask queueを完了まで駆動し、結果objectをJson bridge経由で
native側へ戻して、単一実行ファイルが`42`を出力するところまで検証する。

この経路はnative HIRのPromise constructor/chainテストとは独立しており、実パッケージ内の
class/function closure、CommonJS dependency、QuickJS Promise、Json marshalが同時に成立する
ことを確認する。`JsValue`はQuickJS realm内のcallable/objectをopaque handleとして保持する。
callableの戻り値を別handleとして受け取り、handleを引数として渡し、property取得・設定、
`this`を保ったmethod呼び出し、Promise解決、明示releaseができる。getter/methodのthrowは
Thawのcatchへ伝播する。`.d.ts`でcallable戻り値を判定できるFallback wrapperは自動的に
`JsValue`を返す。

p-limit workloadはpackage exportを通常のdefault importで取得し、`pLimit(2)`が返したlimiterを
別handleに保持し、QuickJS callableのhandleをtaskとして直接渡す。結果PromiseをJSONへ解決する
ため、package内部だけに閉じたwrapperには依存しない。

call signatureを持つinterfaceはnative recordではなく`JsValue`として分類する。これにより
`export default function pLimit(concurrency: number): Limit`をtyped dynamic declarationへ変換し、
利用側は通常の`import pLimit from "p-limit"`と`pLimit(2)`でlimiter handleを得られる。混在呼び出し
はJSON引数の後へ複数handleを追加でき、constructor、Symbol、`undefined`もlive bitmapによって
解放済みslotと区別する。明示releaseに加え、生成プログラム終了時にrealmのhandleを一括解放する。

## 25. Node風の構造化listen error

`node:http`の`server.on("error", callback)`は文字列ではなく、`message`、`code`、`syscall`、
`address`、`port`を持つobjectを渡す。単一実行ファイルの実socketテストで、不正portと
bind競合のcodeをfield accessして検証する。
listenerが一つもない場合はstderrへ`Unhandled 'error' event`を出し、生成実行ファイルの終了値を
非ゼロにする。failure flagは一度だけconsumeされ、listenerがある場合は従来どおり継続する。

## 26. parser-backed ESM／CommonJS graph

依存収集を生テキスト走査からSWC構文木へ移した。静的import、re-export、
literal dynamic import、literal CommonJS requireを同じグラフへ載せ、コメント、
文字列、member callを除外する。literal `import()`はbundle-local requireを
Promise境界内で呼ぶ形へ変換するため、解決失敗も同期throwではなくrejectionになる。

ESMのローカルexportとre-exportは値コピーではなくgetterで公開する。これにより
更新されたexport、namespace import、循環参照時の部分初期化がCommonJS module cache
上でも保たれる。`.cjs`/`.mjs`/`.json`、directory package entry、package
`imports`、exact／single-wildcard `exports`とcondition選択も解決対象に加えた。
混在グラフの実行テストは、live export、循環、`#imports`、JSON、dynamic importを
同時に通してQuickJS上で最終値42を確認する。
## 27. ESM live bindingと非同期module初期化

named/default importの使用箇所を、import時の値コピーではなく依存moduleの
getter参照へAST span単位で置換する。関数引数、block binding、catch bindingの
shadowingとobject shorthandを区別し、再exportされたimportも同じlive参照を使う。

top-level awaitを持つmoduleと、そのmoduleを静的importする上流moduleはasync factoryに
する。CommonJS cacheはexports identityを先に確立し、別の`ready` Promiseで初期化完了を
表す。QuickJSの`loadScript`はentryのreadyをtimer/job queueとともに駆動してからglobal
export aliasを公開する。async dependency cycleはdeadlockさせず、module keyを並べた
明示エラーにする。同期ESM cycleは従来どおり部分初期化cacheで動作する。

JSON moduleは`with { type: "json" }`と旧`assert { type: "json" }`を受理し、その他の
attributeを明示エラーにする。dynamic importは実行時式を`requireAsync(String(expr))`へ
変換し、同一package内のJS／JSON候補をbundle mapへ収録する。同じmoduleを複数回import
しても同じnamespace objectとready Promiseを再利用する。式なしtemplate、括弧、文字列
だけの連結はbundle時に畳み込み、外部packageも通常の依存graphへ載せる。user moduleの
`import.meta.url`はsourceごとの絶対`file://` URLへ変換する。star exportは明示exportを
優先し、異なるbindingの曖昧性をbarrel越しに伝播してimport時に診断する。任意の実行時
文字列から決まる外部packageと、`import.meta.url`以外のmeta propertyはまだ対象外である。

## 北極星: 「npm と同じ感覚で使える」こと

このドキュメントの各章は、実在する npm パッケージを実際に試して
見つかった穴を1つずつ塞いできた記録になっている。最終的に目指す姿は
「`thaw registry add <package>` するだけで、その `package` が npm の
世界でどれだけ普通に書かれていても（内部で複数ファイルに分かれていて
いても、DefinitelyTyped の型を使っていても、`main` フィールドの
書き方が多少雑でも、他パッケージに依存していても）そのまま動く」こと
-- つまり npm を使う感覚と地続きの体験にすること。ESM 専用パッケージも
書いてある順に近い状態で動くようになり、複数パッケージ間の名前衝突も
namespace 修飾構文（15章）で自動的に解決できるようになった。ネイティブ
アドオンはN-API hostで実行でき、package本体内のprebuildとplatform optional
dependencyの双方を自動選択できる（23章）。`<package>@<version>` で
特定バージョンを指定し、実際に解決されたバージョンを記録することも
できるようになった（17章）。Fast path の Marshal アダプタも、
パラメータ側の `(ptr, len)`/フィールド展開までは実装した（18章）。
依存グラフ全体で実際に npm が選んだバージョンを `lock.json` として
記録することもできるようになった（19章、ただし独自の semver
ソルバは書いていない）。より広いN-API／Node互換性など、まだ埋まっていない
穴はある（20章）。優先順位は「実際に試して見つかった
順」で決めていく。
