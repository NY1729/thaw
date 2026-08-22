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

## 7. 今回やらなかったこと（意図的なスコープ外）

- **ネットワーク経由の取得**: `npm install` 相当のダウンロードは存在しない。
  `<registry-dir>` は事前に用意されている前提。
- **バージョン解決**: パッケージ名だけを見る。`package.json`/lockfile
  相当のものは存在しない。
- **ネイティブライブラリのビルド**: `native.a` は事前にビルド済みの
  ものを置く前提。「実際の npm パッケージのネイティブアドオンを
  Thaw 向けにビルドする」パイプラインはまだない。
- **依存関係グラフ**: パッケージ間の依存は `--use` を書いた順序が
  そのまま `loadScript` の呼び出し順序になるだけで、循環検出や
  自動的な依存解決はない。`require(...)` は常にエラーになる
  （6章）。
- **ESM (`import`/`export`) パッケージ**: 6章の CommonJS/UMD 対応は
  `module.exports`/`exports` を書くパッケージのみが対象。`export
  default`/`export { ... }` 構文をそのまま使う ESM 専用パッケージは
  依然として未対応（構文自体が QuickJS-NG のスクリプト評価モードでは
  そのままでは動かない）。
- bridge.md 5章で述べた実際の C ABI（`(ptr, len)` 分割など）に合わせた
  Marshal アダプタ生成は引き続きスコープ外。
