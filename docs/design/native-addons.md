# ネイティブアドオン（`.node`）実行 設計ドキュメント

- ステータス: V1 の最小同期ホストを実装済み。`thaw-napi`、HIR/LLVM、
  registry/bridge/CLI を統合し、実 `.node` のロードと同期呼び出しを検証済み。
- 前提: [thaw-quickjs](../../crates/thaw-quickjs)（Fallback パスの既存実装、
  本ドキュメントの設計はこれと**意図的に並行した構造**にする）、
  [thaw-bridge](../../crates/thaw-bridge)、[thaw-registry](registry.md)
  （特に16章・18章・19章・20章）、[thaw-llvm/hir_codegen.rs](../../crates/thaw-llvm/src/hir_codegen.rs)
  の `compile_call_dynamic`/`MODULE_INIT_SYMBOL` 周り。

追記: `prebuild-install`方式にも対応した。npm install scriptや`node-gyp`は
実行せず、package.jsonのGitHub repositoryと`binary.napi_versions`から
対象資産を選択し、HTTPS取得、安全なtar展開、SHA-256記録を行う。
`sqlite3@5.1.7`の公式N-API v6 Linux x64 prebuildで、実際のモジュール
初期化と`Database`/`Statement`/`Backup`クラス登録、実行ファイルへの埋め込みを
確認済み。これに必要だった`napi_get_global`、`napi_get_property_names`、
`napi_get_uv_event_loop`もホストへ追加した。

さらに`.d.ts`のclass宣言について、継承、constructor、instance/static method、
getter、property、overloadを構造化して抽出するようbridgeを拡張した。overloadは
同名memberへ明示的に記録され、単一signatureと誤認してFast Pathへ流さない。
N-API host側にはexport取得、constructor呼び出し、instance method呼び出しの
不透明handle ABIを追加した。handleはaddonを初期化した同じ`napi_env`に所属し、
method callbackには元instanceを`this`として渡す。自作`NativeBox`の構築とmethod
実行、および実sqlite3 `Database(":memory:")`の構築で検証した。通常のTypeScript
`new Database(...)`／`new sqlite3.Database(...)`は、registryがclass exportを
通常のfunctionと分けて解決し、対象classだけをtyped N-API constructor callへ
書き換えるようになった。LLVMは`DynamicBackend::Napi`のconstructor symbolを
export handle取得とconstructor ABI呼び出しへloweringする。公式sqlite3を取得・
埋め込み、named importから`new Database(":memory:")`を含むTypeScriptを実行
ファイルへコンパイルして、registry削除後にも実instanceを構築できることを確認した。
さらに、外部classのconstructorへ直接代入された変数を追跡し、callbackを含まない
instance methodをtyped N-API callへ書き換える。数値を保持する自作`NativeBox`を
`new NativeBox(42)`で構築し、通常構文の`box.get()`が`42`を返すところまでCLI、
HIR、LLVM、N-API hostを通した実行ファイルE2Eで検証した。getter、static method、
aliasやpropertyを介したinstance追跡は引き続き残課題である。
callback ABIについてはinstance methodにも接続し、0〜2個の動的引数と`Json`／`void`
戻り値を扱う。`.d.ts`の`Error | null`や`any`はこの境界で`Json`へ正規化する。
receiverと同じ永続`napi_env`内にcallback functionを作り、`this`を維持してmethodを
呼ぶ。自作addonの`box.getLater(callback)`を通常構文から実行し、callback結果と
method戻り値の双方を検証した。終了時drainにはdefault libuv loopの`UV_RUN_NOWAIT`も
統合し、実timerの発火とhandle closeを回帰testで確認した。callback methodの直前には
constructor等の先行async workをdrainし、各complete境界で例外を回収して後続callへ
漏らさない。これにより公式sqlite3の実`Database`を構築し、registry削除後の単一実行
ファイルで`close(callback)`の完了通知まで検証できた。
同名instance methodは対応可能な全signatureへ固有symbolを生成し、source rewrite時に
実引数個数と末尾callbackの有無で選択するよう拡張した。inline callbackに加えて、
local変数へ代入したarrow/functionも追跡する。同じ引数個数を持つ非callback overloadも、
number、string、boolean、number array、objectのliteralと、それらを代入したlocal変数を
使って選択する。算術、文字列連結、比較、条件式、template、括弧／type assertion、
primitive変換call、`.length`の結果もlocal変数を通して追跡する。末尾optional parameterは
必須prefixから完全signatureまでの各arityを個別に生成し、実行ファイルE2Eで0引数・1引数
の双方を検証した。number型のrest
parameterはsourceで観測した実引数個数だけを展開するため、固定の最大arityを設けない。
同じE2Eで0要素・3要素をN-API methodへ渡して検証した。user function callも明示的な
戻り値annotation、または全returnから一意に推論できる型を使ってoverloadを選択する。
複数回収集により前方call chainも解決する。parameter依存／競合するreturnと、現在の
native表現を越えるrest要素型は引き続き残課題である。
object literalはfield名と再帰的に推論した型を保持し、property readと同一arityの
structural object overloadを選択する。直線的な`=`代入はlocal変数とnested propertyの
型を更新し、静的文字列のcomputed propertyも扱う。未知またはcompound assignmentは
該当する型情報を破棄する。branch joinとcomputed/spread object literalは引き続き
残課題である。

## 1. 何が難しいのか（おさらい）

registry.md 16章で確認した通り、実際の npm ネイティブアドオン
（`bcrypt`、`utf-8-validate`、`sqlite3`、`sharp`、...）はほぼ例外なく
**N-API**（または NAN 経由で N-API）を使って書かれている。これは V8/Node
専用の C ABI で、Thaw の内部表現とも、単純な `(ptr, len)` 形式の C ABI
（[bridge.md](bridge.md) 5章、registry.md 18章で実装した Marshal アダプタ）
とも全くの別物:

- 値は全て `napi_value` という不透明ハンドルとしてやり取りする
  （実体は呼び出し先=ホスト側が自由に決めてよい、という設計）。
- `.node` ファイルは動的ライブラリで、**Node 本体が実行時に提供する
  `napi_*` という名前の C 関数群にリンクされる**（Node は自分自身を
  `-rdynamic` 相当でビルドしていて、これらのシンボルをプロセス全体に
  見えるようにしている）。`.node` 側は `napi_*` を「未定義の外部シンボル」
  として参照しているだけで、それを解決するのはロード先のプロセス次第。

したがって、Thaw が `.node` を実行できるようにするというのは、**Thaw の
バイナリ自身が Node の代わりに `napi_*` シンボル群を実装して提供する**
ということに等しい。QuickJS-NG 統合のときのような「既存の JS エンジンを
組み込む」というアプローチは使えない -- N-API はエンジンではなく ABI
（呼び出し規約）なので、host 側の実装は自分で書く以外にない。

**信頼境界の変化にも注意**: QuickJS で実行する JS は（相手が悪意ある
パッケージでも）メモリ安全なサンドボックスの中で動く。`dlopen` した
`.node` は Thaw が生成したネイティブバイナリと**同一プロセス内で動く
無制限のネイティブコード**であり、サンドボックスは一切ない。`npm
install --ignore-scripts` の理由（無人での任意コード実行を避ける）と
同種の懸念が、実行時にも形を変えて存在する。

## 2. 目標とする最小スコープ

**やること**: 自分で書いた小さな N-API アドオンをコンパイルし、Thaw から
`dlopen` して、その中の1つの同期関数を実際に呼び出して結果を受け取る。

**やらないこと**（意図的にスコープ外、後日拡張の余地として明示）:

- 非同期 API（`napi_create_async_work`/`napi_queue_async_work` と、その
  裏にある libuv のスレッドプール・イベントループ）。多くの実パッケージの
  `*Async` 系関数はこれに依存するが、`*Sync` 系関数（`bcrypt.hashSync`
  など）は同期のみで完結するので対象に含める。
- Promise 以外の複雑な JS 値（`Symbol`、`Map`、`Set`、`ArrayBuffer` の
  detach など）。
- ハンドルスコープによる本格的な GC（3.3節で単純化する）。
- 複数の Node ABI バージョンへの対応（1バージョンのみ固定でターゲットにする）。
- ファイナライザ（`napi_add_finalizer` 等）。
- 実際の複雑な実パッケージ（`bcrypt` 等）での動作確認 -- まずは自作の
  小さいアドオンで「機構が正しく動く」ことの証明を優先する。実パッケージは
  N-API 関数の呼び出しパターンが多様なため、howeverこの V1 の先に
  「呼んだら未実装の `napi_*` に当たってエラーになる」を1つずつ潰していく、
  という registry.md と同じ反復が必要になる想定。

## 3. アーキテクチャ

### 3.1 新しいクレート: `thaw-napi`

`thaw-quickjs` と同格の、独立した crate として作る（QuickJS とは無関係な
別の実行バックエンドなので、`thaw-quickjs` に相乗りさせない）。公開する
C ABI は **意図的に `thaw-quickjs` と同じ形にする**:

```rust
// crates/thaw-napi/src/lib.rs
#[no_mangle]
pub extern "C" fn thaw_napi_load(path: *const c_char) -> u8;
// path の .node ファイルを dlopen し、モジュール初期化関数を呼び出し、
// エクスポートされた関数を名前で引けるように内部テーブルに登録する。
// 成功なら 1、失敗なら 0（thaw_js_load と同じ規約）。

#[no_mangle]
pub extern "C" fn thaw_napi_call(func_name: *const c_char, args_json: *const c_char) -> *const c_char;
// 登録済みの関数を JSON 引数配列で呼び出し、JSON 文字列として結果を返す。
// 失敗時は {"__thaw_error__": "..."} を返す（thaw_js_call と同じ規約）。
```

**この対称性が最重要の設計判断**: HIR/codegen 側（`hir_codegen.rs` の
`compile_call_dynamic`、`callDynamic` という特別扱いされる呼び出し名）は
すでに「関数名と JSON 引数を渡し、JSON 文字列を受け取る」という抽象度で
実装されている。バックエンドが QuickJS か N-API ホストかは、
**`callDynamic` を呼ぶ側からは区別する必要がない**。実際には
`callDynamic` 自体を分岐させるのではなく、`thaw-bridge::generate_shim`
が Fallback 関数ごとに「QuickJS 経由か N-API ホスト経由か」をあらかじめ
知っていて、生成する呼び出し先の関数名（`callDynamic` vs 新しい
`callNativeAddon` のような別名）を出し分ける形にする -- 呼び出し規約
そのものは共通化しつつ、実行時にどちらのテーブルを引くかは静的に決まる
ようにする（実行時分岐よりコンパイル時に決まる方が、Thaw のミニマルな
compiler投資方針に合う）。

### 3.2 `napi_value`/`napi_env` の内部表現

N-API は両方とも不透明ポインタとして定義されている（ホスト実装が中身を
自由に決めてよい）:

```rust
// 概念的なイメージ。実際の内部enumはもっと種類が要る。
enum NapiValueData {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Object(HashMap<String, NapiValue>), // 順序は問わない、Fast pathのような固定レイアウトは不要
    Array(Vec<NapiValue>),
    Buffer(Vec<u8>),
    Function(RegisteredCallback), // アドオン自身が napi_create_function で作った関数
}

type NapiValue = *mut NapiValueData; // Box::into_raw で確保、そのままポインタとして渡す
```

`napi_env` も同様に不透明ポインタ。中身は「現在の呼び出しで作られた
`NapiValueData` を全部集めておくアリーナ」＋「保留中の例外（あれば）」
くらいで十分（3.3節）。

### 3.3 スコープ管理の単純化（本物の N-API との差分）

本物の N-API はハンドルスコープ（`napi_open_handle_scope`/
`napi_close_handle_scope`、V8 の GC と連動）で `napi_value` の生存期間を
管理する。Thaw に GC は無い（アリーナベース、[[project_thaw_overview]]
と同じ設計哲学）ので、ここは大胆に単純化してよい:

- `napi_open_handle_scope`/`napi_close_handle_scope`/
  `napi_open_escapable_handle_scope`/`napi_close_escapable_handle_scope`/
  `napi_escape_handle` は **全部 no-op** にする（呼ばれたことにして
  成功を返すだけ）。
- `napi_env` に1回の `thaw_napi_call` 呼び出し中に作られた全ての
  `NapiValueData` の `Box` ポインタを push だけしておき、呼び出しが
  終わったら（結果を JSON に変換し終えたら）まとめて `Box::from_raw` で
  回収して drop する。これは QuickJS 統合が「1回の `thaw_js_call` の中で
  完結する」のと同じ寿命モデル。

### 3.4 モジュール登録

実際の N-API アドオンは `NAPI_MODULE(NODE_GYP_MODULE_NAME, Init)` という
マクロで自分自身を登録する。このマクロは実際には「共有ライブラリが
ロードされた瞬間に走る C++ の静的コンストラクタ」を生成し、その中で
`napi_module_register(&module)` という関数（これも `napi_*` の1つ、ホスト
=Thaw が提供する）を呼ぶ。つまり:

1. Thaw 側が `napi_module_register` を実装し、渡された `napi_module`
   構造体（`nm_register_func` フィールドに実際の init 関数ポインタが
   入っている）を **スレッドローカルな「登録待ちモジュール」の1個の
   スロット** に保存する（QuickJS 統合の `thread_local! { static JS: ... }`
   と同じパターン）。
2. `thaw_napi_load(path)` は `dlopen(path, RTLD_NOW)` する --
   この呼び出し自体の**副作用として** 1. の静的コンストラクタが走り、
   スロットにモジュールが登録される。
3. `dlopen` から戻ってきたら、スロットを確認し、登録された
   `nm_register_func`（シグネチャは
   `extern "C" fn(napi_env, napi_value) -> napi_value`）を、空の
   `napi_create_object` で作った `exports` を渡して直接呼び出す。
4. 戻ってきた `napi_value`（初期化済みの `exports` オブジェクト）を
   舐めて、`.d.ts` から分かっている Fallback 関数名ごとに
   `napi_get_named_property` で引き当て、「関数名 -> `napi_value`
   （中身は `Function` バリアント）」の内部テーブルに登録する
   （`thaw_napi_call` はこのテーブルを引く）。

`dlopen` で解決される `napi_*` シンボルは、Thaw の実行バイナリ自身が
`#[no_mangle] pub extern "C" fn napi_xxx(...)` として**エクスポート**して
いる必要がある。Rust の cdylib/bin ターゲットではデフォルトでシンボルが
外部から見えないことがあるため、リンカフラグ（Linux なら
`-Wl,--export-dynamic`、`thaw-cli` が `cc` を呼ぶ最終リンクコマンドに
追加する）が必要になる可能性が高い -- 実装時に最初にリンクエラー/
`undefined symbol` で気づく類の問題なので、"実際に試して見つかった穴を
塞ぐ" といういつもの手順で確認すること。

### 3.5 最小限必要な `napi_*` 関数一覧（同期のみ）

自作の小さいテストアドオンから逆算した、最初の一歩に要る関数群
（本物の Node の `node_api.h`/`js_native_api.h` のシグネチャに厳密に
一致させる -- でないと `.node` 側のシンボル解決自体が失敗する）:

**モジュール登録**
- `napi_module_register`

**値の作成**
- `napi_get_undefined` / `napi_get_null` / `napi_get_boolean`
- `napi_create_double`
- `napi_create_string_utf8`
- `napi_create_object`
- `napi_create_array` / `napi_create_array_with_length`
- `napi_create_buffer` / `napi_create_buffer_copy`（`Buffer` は実パッケージの
  引数/戻り値で非常によく使われる -- 優先度高め）

**値の読み取り**
- `napi_typeof`
- `napi_get_value_double`
- `napi_get_value_bool`
- `napi_get_value_string_utf8`
- `napi_get_buffer_info`
- `napi_is_array` / `napi_get_array_length`

**オブジェクト操作**
- `napi_set_named_property` / `napi_get_named_property`
- `napi_set_element` / `napi_get_element`

**関数関連（アドオン自身が関数を作る/ホストが呼ぶ）**
- `napi_create_function`
- `napi_get_cb_info`（コールバック内から引数・`this`・`data` を取り出す）
- `napi_call_function`（アドオン側が JS 関数を呼び返すケース -- 優先度低、
  無くても動くアドオンは多い）

**エラー処理**
- `napi_throw_error` / `napi_throw_type_error`
- `napi_is_exception_pending` / `napi_get_and_clear_last_exception`
- `napi_create_error`

**バージョン確認（アドオン側が起動時にチェックすることがある）**
- `napi_get_version`
- `napi_get_node_version`

**スコープ（3.3節の通り no-op）**
- `napi_open_handle_scope` / `napi_close_handle_scope`
- `napi_open_escapable_handle_scope` / `napi_close_escapable_handle_scope` /
  `napi_escape_handle`

一覧にない `napi_*`（`napi_create_async_work` 系、`napi_wrap`/
`napi_unwrap`、`napi_define_class` 等）は、実装せずビルドし、実際に
リンク/実行して「未定義シンボル」または「呼ばれたら明示的にエラーを返す
スタブ」で検出する -- registry.md の一貫した方針（実際に使われた時点で
1つずつ追加する）をここでも踏襲する。

## 4. `thaw-registry`: `.node` の入手経路

registry.md 16章時点では、レジストリは `.node`/ネイティブビルド成果物を
一切生成・保存していない。ここを埋める2つの現実的な経路:

1. **プリビルド済みバイナリの取得**（優先度高）: `node-gyp-build`/
   `prebuild-install` に対応した実パッケージの多くは、GitHub Releases に
   プラットフォーム別のプリビルド `.node` を公開しており、`--ignore-scripts`
   下でも `node-gyp-build` 自身のロジック（registry.md 16章で実際に
   Thaw の `fs`/`path`/`os` polyfill 越しに動くようにした、あの
   `node-gyp-build.js`）が正しい URL を計算できる。この URL に対して
   Thaw 側（Rust）で実際に HTTP フェッチし、ダウンロードした `.node` を
   `registry_dir/<package>/native.node` として保存する、という経路。
   C++ ツールチェインが不要な分、現実的な npm パッケージの多くをこれで
   カバーできる可能性がある。
2. **`node-gyp rebuild` の実行**（優先度低、フォールバック）: プリビルドが
   無い場合、パッケージ自身の `binding.gyp` を使って実際にソースから
   ビルドする。ただし: (a) システムに C/C++ ツールチェインと Node の
   ヘッダ一式が要る、(b) `--ignore-scripts` の元々の理由（無人での
   任意コード実行の回避）と本質的に同じ懸念が「ビルドスクリプトの実行」
   という形でそのまま当てはまる -- ユーザーの明示的な同意（フラグ、
   例えば `thaw registry add <pkg> --allow-native-build`）なしに自動で
   行うべきではない。

いずれの経路でも、`AddedPackage`/`ResolvedPackage` に
`native_addon: Option<PathBuf>` のような新しいフィールドを足す必要がある
（既存の `native_lib`＝Fast path 用 `native.a` とは別物として区別する --
呼び出し規約が全く違うため、`thaw-bridge` 側で別の分類・別のシム生成が
要る）。

## 5. `thaw-bridge`: 分類とシム生成

現状の `classify`/`classify_all` は Fast path / Fallback の二値。ここに
「Fallback だが実体が `.node`（N-API ホスト経由）」という区別を足す
必要がある。ただし **`.d.ts` の型シグネチャだけからは判別できない**
（Fallback の中に QuickJS 版と N-API 版が混在しうる）ので、判断材料は
`ResolvedPackage::native_addon`（4章）の有無 -- 実際にリンクできるかどうか
で Fast path を後から Fallback に格下げする既存の
`native_lib_available`（bridge.md 4.4節）と全く同じパターンを、
「N-API ホスト経由にするかどうか」の判断にもう一つ追加する形になる。

生成するシムの呼び出し先関数名を、QuickJS 版の `callDynamic` と分けて
`callNativeAddon`（3.1節）にするだけで、それ以外の shim 生成ロジック
（JSON 引数の組み立てなど）は `callDynamic` 用のものをほぼそのまま流用
できるはず -- 新しい `HirExpr` は増やさず、`hir_codegen.rs` に
`compile_call_dynamic` と対になる `compile_call_native_addon` を足す形。

## 6. `__thaw_module_init` との統合

registry.md 4章の `__thaw_module_init`/`generate_module_init` は
「`loadScript(...)` を並べる」という形で QuickJS へのロードを表現していた。
N-API ホスト向けには、同じ関数の中に `loadNativeAddon("<path>")` という
新しい特別扱い関数（`loadScript` と対になる形）を増やし、
`thaw_napi_load` を呼ぶようにする -- これも新しい構文は増やさず、
「特別扱いする関数名」を1つ足すだけで済む、registry.md 4章と同じやり方。

## 7. 検証計画（最初の一歩）

1. 自分で最小の N-API アドオンのソース（C か C++ どちらでもよいが、
   `node_api.h` を素で使う C の方が依存が少なく単純）を書く: 例えば
   `napi_value Add(napi_env env, napi_callback_info info)` が2つの
   `number` 引数を取って和を返すだけの関数を1つエクスポートする。
   Node の実際のヘッダ（`node_api.h`/`js_native_api.h`、npm の
   `node-addon-api`/Node 本体のソースから入手可能）に対して `cc -shared`
   でコンパイルし、`.node` 拡張子で保存する（中身はただの `.so`）。
2. `thaw-napi` クレートに `thaw_napi_load`/`thaw_napi_call` を実装し、
   1. で作った `.node` を実際に `dlopen` して `Add` を呼び出し、正しい
   和が返ることを確認する単体テスト（`thaw-quickjs` の
   `binds_esm_default_export_under_the_fallback_name` 等と同じ、実行系を
   本当に動かして確認するテストの書き方を踏襲）。
3. ここまで機構が実証できたら、4〜6章の統合（`thaw-registry`/
   `thaw-bridge`/`__thaw_module_init`）に進み、`thaw build --use
   <package>` から自作アドオンをエンドツーエンドで呼べることを確認する。
4. 実際の実パッケージ（`utf-8-validate`/`bcrypt` の `*Sync` 系関数）は
   3. まで通ってから初めて試す -- registry.md のいつもの手順
   （実パッケージ→エラー→1つ塞ぐ、を繰り返す）が、ここでも
   `napi_*` 関数1個ずつという単位で発生する見込み。

## 8. 今回書かなかったこと（意図的なスコープ外、再掲）

2章の一覧の通り。特に強調すべき点: **非同期 API 抜きでも `*Sync` 系関数は
現実的に多くカバーできる**（`bcrypt.hashSync`/`compareSync`、
`better-sqlite3` の同期 API 中心の設計など）。まず同期のみで最大公約数を
取りに行く、という優先順位が妥当と考える。

## 9. 実装結果

`crates/thaw-napi` は `napi_register_module_v1` と
`napi_module_register` の両登録方式を受け付け、数値・文字列・真偽値・
object・array・Buffer、同期callback、例外、no-op handle scope を提供する。
呼び出し引数と結果は QuickJS fallback と同じ JSON 境界を使い、失敗は
`ThawResult { value, error }` を通じて既存の `try/catch/finally` に入る。

registry は `native.a` と区別して `native.node` を検出する。bridge は
`callNativeAddon` wrapper と `__thaw_native_module_init` を生成し、LLVM は
`thaw_napi_load`/`thaw_napi_call_result` を呼ぶ。CLI の最終リンクでは
`thaw-napi`、`libdl`、`--export-dynamic` を追加し、addon が参照する
`napi_*` symbol を解決可能にする。

V1 は Linux のローカル `native.node` を対象とする。`node-gyp`、非同期work、
class/wrap/finalizer、および実パッケージ固有APIは引き続きスコープ外である。

## 10. 実パッケージ互換性

`utf-8-validate@6.0.6` の公式 Linux x64 prebuild を実行確認した。このaddonが
参照するのは `napi_module_register`、`napi_create_function`、
`napi_get_cb_info`、`napi_get_buffer_info`、`napi_get_boolean` の5つで、V1
ホストの範囲内だった。一方、初期化関数はexports objectではなくcallback
functionそのものを返すため、単一関数の`.d.ts`名をroot export名として
`thaw_napi_load_named`へ渡す処理を追加した。

また、QuickJS/ThawとのJSON境界ではNodeのBuffer JSON表現
`{"type":"Buffer","data":[...]}`を検出し、`Value::Buffer`へ変換する。
公式prebuildに対して有効な4-byte UTF-8と不正な`0xff`を渡し、それぞれ
`true`/`false`になることをhost単体と`thaw build --use`の両方で検証する。
回帰テストは再配布バイナリをrepositoryへ含めず、
`THAW_UTF8_VALIDATE_NODE`にprebuildのpathが指定された環境で実行する。

## 11. 同梱prebuildの自動取得

`thaw registry add`が`npm install --ignore-scripts`で取得したpackage内の
`prebuilds/<platform>-<arch>/`を探索し、現在のOS、CPU、libcに一致する
`.node`を`native.node`としてregistryへコピーする。Linuxではglibc用と
`.musl.node`を区別し、macOSのRust target名`macos`はnpm慣習の`darwin`へ、
`x86_64`/`aarch64`は`x64`/`arm64`へ対応付ける。

選択結果は`native-addon.json`へ元package内の相対path、SHA-256、platform、
arch、libcとともに保存する。再追加時は以前の`native.node`とmetadataを先に
除去するため、version/targetが変わっても古いbinaryは残らない。prebuildsが
存在しない場合は従来通りJSのみ、存在するが一致しない場合は利用可能targetを
診断に表示してJS fallbackを維持する。install scriptや`node-gyp`は実行しない。

network統合テストは`THAW_RUN_NPM_INTEGRATION=1`で有効化し、
`utf-8-validate@6.0.6`について`registry add`から`build --use`、有効/不正UTF-8
の実行までを一続きで検証する。

## 12. 単一実行ファイルへの埋め込み

CLIは選択済み`native.node`をビルド時に読み、16進データとして生成HIRへ
埋め込む。Linuxの実行時は`thaw_napi_load_embedded_hex`が復号し、匿名
`memfd`へ書き込んで`/proc/self/fd/<fd>`から`dlopen`する。このため配布先に
registry、`native-addon.json`、元の`.node`をコピーする必要はない。

統合テストはリンク完了後にregistryディレクトリを削除してから生成物を起動し、
埋め込まれたaddonだけで呼び出せることを確認する。非Linuxでは互換経路として
一時ファイルへ展開するが、配布成果物そのものは同じく実行ファイル1個である。
OSの標準dynamic loader、libc、libm、libdlまで静的同梱する保証とは分けて扱う。

Linuxでは`thaw build --static`を指定すると、最終リンクに`-static`を加え、
生成ELFに`PT_INTERP`が存在しないことまでCLI自身が検査する。`libc.a`、
`libm.a`、`libdl.a`がない環境ではコンパイル開始前に導入方法を含む診断を返す。
手動`--link`で`.so`/`.dylib`を渡す構成は完全静的という契約に反するため拒否する。
同じ理由でN-API `.node`を含むbuildも`--static`では拒否し、JavaScript
fallbackまたは`native.a` backendの利用を診断する。通常の単一ファイルbuildは
引き続きaddon bytesを内包するが、完全静的ELFとは異なる配布モードである。

`thaw inspect <executable>`は埋め込みmetadataを読み、ELF architecture、
`PT_INTERP`に基づくlinkage、npm package一覧、QuickJS／N-API有無を表示する。
`THAW_RUN_CONTAINER_INTEGRATION=1`で有効になるE2Eは、成果物1個だけをread-onlyで
networkなしのFedora containerへmountし、完全静的binaryの起動を検証する。

## 13. 非同期work V1

`napi_create_async_work`、`napi_queue_async_work`、`napi_delete_async_work`を実装する。
`execute` callbackは共有worker poolで実行し、完了したworkはthread-safe queueへ
送る。N-APIを利用する生成実行ファイルはuser `main`の終了後にqueueをdrainし、
`complete` callbackをmain threadで実行してからprocessを終了する。complete内からの
`napi_delete_async_work`も許可し、同じworkの二重queueや実行中のdeleteはstatus errorに
する。unit testでthread境界を、実際のC製`.node`をリンクするE2Eで生成entry pointからの
drainを検証する。

`napi_cancel_async_work`は、共有queueで待機中のworkだけを取り除き、executeを呼ばずに
`napi_cancelled` statusでcompleteへ渡す。workerによる取り出しとcancelは同じlockで
直列化されるため、execute開始後のcancelは`napi_generic_failure`になる。workerはCPU
並列度を上限4に丸めた共有poolであり、workごとにはthreadを生成しない。libuvそのもの
とのqueue共有や優先度制御は対象外である。

## 14. bcrypt実package検証

`bcrypt@6.0.0`の公式Linux x64 glibc prebuildを対象に、`nm -D`で要求される
Node-API surfaceを列挙した。async workに加えてreference、FunctionへのSymbol
property、汎用property操作、external、Latin-1文字列、callback scope、error API、
Buffer/typed-array判定を実装した。特にnode-addon-apiはcallback dataをFunctionの
private Symbol propertyとして管理するため、FunctionもObjectと同様にpropertyを
保持する必要がある。

`THAW_BCRYPT_NODE`で有効になる実package testは、公式`.node`をロードして
`gen_salt_sync`と`encrypt_sync`を実行し、さらにcallbackを渡した`gen_salt`を共有
worker poolで実行する。生成されたsaltがmain-thread complete callbackへ返るところまで
検証する。

## 15. コンパイル済みcallback bridge

`callNativeAddonWithCallback(name, args, callback)`は、従来のJson引数に生成コードの
`(Json, Json) => Json` closureを追加する。LLVMはclosure環境をcontextとして渡し、
N-API hostが作ったFunction値の呼び出しをC ABI adapterで受ける。adapterはerror/result
JSON文字列を`thaw_json_parse`してclosureをmain threadで呼ぶ。非同期workが残る間は
呼び出し用`Env`とFunction/referenceをhostが保持し、drain完了後にまとめて解放する。

実C addon E2Eは同一callbackの複数回呼び出しとprocess終了前drainを検証する。
`THAW_BCRYPT_NODE`付きCLI E2Eは公式prebuildを実行ファイルへ埋め込み、registry削除後に
`gen_salt`、`encrypt`、`compare`を並行実行する。成功値だけでなく不正saltのerror引数も
生成lambdaへ戻ることを検査する。待機中workのcancelとcancelled statusはhost unit test
で競合を固定して検証済みである。

## 16. classベースaddon

`napi_define_class`はconstructor Functionとprototype Objectを作り、instance propertyと
`napi_static` propertyをそれぞれprototype／constructorへ定義する。
`napi_new_instance`はprototype methodとaccessorを引き継ぐObjectを作成し、constructor
callbackには`this`と`new_target`を渡す。`napi_instanceof`は生成元constructorをEnv内で
追跡する。

native instance dataは`napi_wrap`／`napi_unwrap`／`napi_remove_wrap`で管理する。同じ
Objectへの二重wrapは拒否し、removeされなかったdataのfinalizerはObjectを所有するEnvの
破棄時に一度だけ呼ぶ。weak reference用に返される`napi_ref`とreference count操作、
external値の取得も実装する。

実C addon testは`NativeBox` classを定義し、constructorでC heap dataをwrapする。
prototype methodとgetter accessorの双方がunwrapした値を読み、`napi_new_instance`と
`napi_instanceof`を経由して結果を返す。呼び出しEnv破棄後にはfinalizer countが1になる
ことまで検査する。継承チェーン、property attributesの完全なdescriptor semantics、
GCのweak-reference clearingは今後の拡張対象である。

## 17. serialport実package検証

`@serialport/bindings-cpp@12.0.1`の公式Linux x64 glibc prebuildを、
`THAW_SERIALPORT_NODE`で有効になるhost testでロードする。addonは
`node-addon-api`の`ObjectWrap`を使って`Poller` classを定義するため、手書きC fixture
だけでなく実packageのclass exportまで検証できる。

この検証で`napi_add_finalizer`、environment単位の
`napi_set_instance_data`／`napi_get_instance_data`、coercion、int64、named-property
query、`napi_make_callback`の不足が判明し、hostへ追加した。serialportはNode本体が
公開する`uv_poll_*`等も直接参照するため、Linuxではaddonの解決前にsystem
`libuv.so.1`をglobal visibilityでロードする。

## 18. thread-safe function

`napi_create_threadsafe_function`はaddon workerから渡されたopaque dataをmutex保護queueへ
積み、`thaw_napi_run_async_work`が生成実行ファイルのmain thread上で`call_js` callbackを
実行する。bounded queueはnonblocking時に`napi_queue_full`を返し、blocking時は空きを待つ。
main thread自身が満杯queueをblocking投入しようとした場合は`napi_would_deadlock`を返す。

producer数は`napi_acquire_threadsafe_function`／`napi_release_threadsafe_function`で管理する。
最後のrelease後にqueueをdrainしてfinalizerを一度だけ実行し、abort releaseでは残ったdataを
null env／null JS functionの`call_js`へ渡してnative cleanupを可能にする。ref/unrefは生成実行
ファイル終了時のdrain待機対象を切り替える。thread-safe functionを作ったcallback Envは
finalize完了まで`pending_call_envs`に保持する。

host testはworker threadからのblocking投入、callbackのmain-thread実行、queue full、
deadlock防止、abort cleanup、acquire/release、ref/unref、finalizerを固定して検証する。
加えて実C addonを`-pthread`でbuildし、native pthreadからTSFNへ投入した値が生成callback
bridgeを通ってmain thread上で`42`として返るところまでE2E検証する。

## 19. Parcel Watcher実package検証

`@parcel/watcher@2.5.1`はasync-work、deferred Promise、TSFNを同時に使う。hostは
`napi_create_promise`／`napi_resolve_deferred`／`napi_reject_deferred`を値モデルへ追加し、
`thaw_napi_poll_async_work`で待機せずready callbackだけをmain thread上で進める。
同期bridgeからPromiseを返した場合はnative async-workがsettleするまでpollし、resolved値を
JSONへ変換する。`napi_strict_equals`と`napi_fatal_exception`も実prebuild要求から追加した。

`THAW_PARCEL_WATCHER_NODE`付きhost E2Eは一時directoryをsubscribeし、実ファイル作成を
inotify backendが検出してTSFN callbackへ返すこと、同じcallback identityによるunsubscribe、
両操作のPromise解決まで検証する。snapshot APIもPromise bridge経由で実ファイルを生成する。

Parcelのbinaryは本体package内の`prebuilds/`ではなく、
`@parcel/watcher-linux-x64-glibc`のようなplatform optional dependencyへ分離される。
registryは現在のOS／architecture／libc suffixに一致するoptional dependencyの`main`が
`.node`なら自動選択し、従来と同じ`native.node`／`native-addon.json`へ格納する。
`THAW_RUN_NPM_INTEGRATION=1`のCLI E2Eは実npm packageを取得し、snapshot実行ファイルをbuild、
registry削除後の単独実行まで確認する。

## 20. callback identityと実行中poll

`callNativeAddonWithCallback`のLLVM adapterは呼び出し地点ごとに別symbolになるが、同じ
closure valueのcontext pointerは安定している。hostはcontextをidentity keyとして生成済み
N-API Functionを保持し、subscribeとunsubscribeが異なる呼び出し地点でも同じ`napi_value`を
受け取れるようにする。最後のTSFNがfinalizeされた時点でcallback cacheと保持Envを破棄する。

`pollNativeAddonEvents(): number`は`thaw_napi_poll_async_work`を呼び、現在readyなasync completion
とTSFN callbackだけをmain threadで処理して待たずに返る。Parcel CLI E2Eは生成された単一
実行ファイル内でsubscribeし、`node:fs`でファイルを作り、event到着までpollし、同じclosureで
unsubscribeする。registry削除後の実行、event受信、正常終了まで検証する。

## 21. cleanup、unload、fatal callback

`napi_add_env_cleanup_hook`／`napi_remove_env_cleanup_hook`を実装し、Env破棄時に登録と逆順で
cleanup hookを実行する。`thaw_napi_unload_all`はactive async-workまたはlive TSFNがあれば拒否
する。安全に停止済みならcallback cacheとEnvを先に破棄してcleanup hook／finalizerをaddon
codeがmappedな間に実行し、その後library handleを逆順に`dlclose`する。生成mainは最終drain後
にunloadを呼ぶ。

`napi_fatal_exception`は診断をstderrへ出し、process failure flagを設定する。生成mainはcleanup
後にflagを一度だけ取得し、未処理callback例外があれば終了status 1を返す。cleanup hookのLIFO、
remove、active TSFN中のunload拒否、fatal statusのconsume-onceをhost testで固定する。
