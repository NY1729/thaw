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

1. `main` から開始し、各ファイルの生テキストを正規表現ではなく
   単純な文字列走査で `require('./x')`/`require("../y")` パターンだけ
   探す（この Rust コードベースには汎用 JS パーサーがなく、
   thaw-parser は `.d.ts`/`.ts` 向けの TS パーサーのため）。相対パス
   （`./`/`../` で始まる）だけを対象にする -- `require('lodash')` の
   ような裸のパッケージ名は意図的にそのまま残す。
2. 各相対 require の相手先を、既存の `resolve_module_path`（7章、
   元々は `main` フィールド解決用だった関数を汎用化）で解決し、
   まだ見ていないファイルならキューに積む。解決できない場合
   （`.json` を require しているなど）はそのファイルの依存マップに
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

## 14. その他、今回やらなかったこと（意図的なスコープ外）

- **名前衝突の自動解決**: 13章の通り検出してエラーにするところまでは
  実装したが、`qs.stringify`/`hoek.stringify` のような namespace
  修飾付きの呼び出し構文（自動リネームや `import` 相当の仕組み）は
  まだない -- コンパイラ本体（パーサー/HIR）への追加投資が要るため、
  意図的に見送っている。
- **バージョン解決**: パッケージ名だけを見る。`package.json`/lockfile
  相当のものは存在しない。`npm install` は常に最新版を取得する
  （`@types/*` へのフォールバックも同様、本体パッケージとの
  バージョン整合は見ていない）。
- **ネイティブライブラリのビルド**: `native.a` は事前にビルド済みの
  ものを置く前提。「実際の npm パッケージのネイティブアドオンを
  Thaw 向けにビルドする」パイプラインはまだない。
- **ESM (`import`/`export`) パッケージ**: 6章の CommonJS/UMD 対応は
  `module.exports`/`exports` を書くパッケージのみが対象。`export
  default`/`export { ... }` 構文をそのまま使う ESM 専用パッケージは
  依然として未対応（構文自体が QuickJS-NG のスクリプト評価モードでは
  そのままでは動かない）。
- namespace 内で宣言された `interface`/`type`（9章末尾）。
- **その他のプラットフォームグローバル**: `Buffer`/`URL` 以外にも
  `process`/`TextEncoder`/`setTimeout` など、実際に参照する
  パッケージにぶつかった時点で都度追加していく前提（12章と同じ方針）。
  網羅的な対応表は用意していない。
- bridge.md 5章で述べた実際の C ABI（`(ptr, len)` 分割など）に合わせた
  Marshal アダプタ生成は引き続きスコープ外。

## 北極星: 「npm と同じ感覚で使える」こと

このドキュメントの各章は、実在する npm パッケージを実際に試して
見つかった穴を1つずつ塞いできた記録になっている。最終的に目指す姿は
「`thaw registry add <package>` するだけで、その `package` が npm の
世界でどれだけ普通に書かれていても（内部で複数ファイルに分かれていて
いても、DefinitelyTyped の型を使っていても、`main` フィールドの
書き方が多少雑でも、他パッケージに依存していても）そのまま動く」こと
-- つまり npm を使う感覚と地続きの体験にすること。ESM 専用パッケージや
未対応のプラットフォームグローバルをはじめ、まだ埋まっていない穴は
多いが、優先順位は
「実際に試して見つかった順」で決めていく。
