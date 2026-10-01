# 全体レビュー指摘の修正計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 全体レビューで確認した不具合を、共通原因ごとに修正し、メモリ安全性とJavaScript／Node.js互換性を回復する。

**Architecture:** 既存のHIR、LLVMコード生成、JIT、ネイティブランタイム、互換レイヤーの責任分担を維持する。共有経路に原因がある場合はそこで一度修正し、呼び出し元へ個別の回避処理を散らさない。ABI変更が必要な修正だけは、宣言・生成コード・ランタイムを同じ変更単位で揃える。

**Tech Stack:** Rust、TypeScript／JavaScript、LLVM、QuickJS、Node-API、既存のCargoワークスペース。

**Spec:** このチャットの確定済みレビュー報告。確認範囲の記録は `/tmp/thaw-review-coverage.json`。この記録は指摘一覧ではなくファイル確認履歴であり、実装開始時に報告本文との対応表を作る。

## 制約

- 本計画の作成では製品コードを変更しない。修正実装は別の依頼として扱う。
- ユーザーがテストを担当する。エージェントはテスト、ビルド、実行プローブを実行しない。下記の確認項目と必要な回帰ケースをユーザーへ引き渡す。
- 現在の作業ツリーには多数の変更がある。reset、stash、広範囲のcheckout、無差別なgit add／commitを行わない。
- 実装前に指摘箇所を現行コードで再確認する。既に修正済みなら追加修正せず、根拠を対応表に残す。
- 新しい依存、共通フレームワーク、全体リファクタリングは追加しない。既存ヘルパーと標準機能を優先する。
- 不確かな候補は修正対象にしない。確定指摘、重複、条件付き指摘、保留を区別する。
- ICU生成データは構造確認済みだが、全CLDR値の独立検証はしていない。生成データの手編集は計画に含めない。

## レビューで重点確認する入力

1. null／undefined／省略引数と副作用を伴う式：Task 4・7で値と評価回数の両方を確認。
2. Optional／Union／Tuple／voidを含む型：Task 2・5・6でABIと推論を確認。
3. lone surrogate、NUL、数値の範囲外・非有限値：Task 8・9で表現を失わないことを確認。
4. 並行処理、途中失敗、解放後の再利用：Task 1・3・11で所有権と状態復元を確認。
5. 入れ子のimport、exports条件、パッケージ名衝突：Task 12・13で解決範囲と書き換え範囲を確認。

## 実装順と並列化

| 段階 | タスク | 着手条件 | 完了条件 |
|---|---|---|---|
| 0 | 0：指摘の対応表と作業基準 | なし | 全報告を一意の項目へ対応付ける |
| 1 | 1〜3：安全性・所有権・ABI | Task 0 | 不正参照、解放順、レイアウトの指摘を閉じる |
| 2 | 4〜6：評価順・型・コード生成 | ABIに関係するTask 2 | HIRとJITの意味論、型生成を揃える |
| 3 | 7〜11：APIの意味論 | 関連する段階1・2 | API単位の確認ケースをユーザーが確認 |
| 4 | 12〜14：解決・配信・最終照合 | 個別にはTask 0から可 | 対応表の未処理をゼロにする |

Task 1〜3は共有ABI・所有権を変更する可能性があるため、関連箇所を同時編集しない。Task 4と5も共通HIR／JITファイルの担当を固定する。独立した日時、文字列、レジストリ、サンプルはSolで分担可能。実装担当と再レビュー担当を分け、最終確認は変更差分全体で行う。大きな一括PRではなく、各タスクの独立した修正単位でレビューする。

## Task 0：指摘の対応表と作業基準を確定する

**Files:** この計画、レビュー報告に挙げた現行ファイル。新規の製品ファイルなし。
**Interface:** 後続タスクへ、指摘ID・現行箇所・共通原因・担当タスク・状態・ユーザー確認結果の対応表を渡す。

- [ ] チャットの全レビュー報告を対応表へ移す。過去の番号を新しい連番で上書きせず、参照元を残す。
- [ ] 同一原因の指摘を束ねる。regex／templateのarenaメタデータ寿命、private／identity列挙、host所有値のリークは関連指摘を同じ修正単位へ置く。
- [ ] 撤回済み候補を除外する：FFI setterの古い署名、MapのNUL切り捨て、String.searchの二重評価など。nested functionのthisとFFI absent payloadの契約は未確定として除外する。
- [ ] ユーザーの未コミット変更を記録し、修正済み指摘と未修正指摘を現行コードで分ける。
- [ ] 各後続タスクの対象を対応表と照合する。過去報告にここで列挙していない確定指摘があれば、既存タスクへ割り当てるか独立タスクを追加する。未割当がある間は実装完了としない。

## Task 1：arena・ネイティブ所有権・HTTPヘッダーの寿命

**Files:** `crates/thaw-arena/src/{lib,strings}.rs`、`crates/thaw-napi/src/napi/module_host.rs`、`crates/thaw-quickjs/src/quickjs/api.rs`、`crates/thaw-runtime/src/runtime/native_values/{regex,template_strings}.rs`、HTTPヘッダー関連の既報箇所。
**Interface:** 既存公開ABIを維持し、所有／借用、環境寿命、arenaリセットの契約を明示する。

- [ ] NAPI初期化失敗時のdlcloseとEnv破棄の順序を、ファイナライザがロード済みコードを参照できる順に直す。
- [ ] NAPIコールバックで一時Envを使う経路を、値の所属環境と整合するよう修正する。
- [ ] arenaリセット後も残るメタデータをarenaの寿命と揃える。regexとtemplate双方を確認する。
- [ ] HTTP getHeaderの返却値寿命を修正し、ヘッダー置換・重複Content-Lengthの既報指摘を同じAPI内で閉じる。
- [ ] ユーザー確認項目：初期化失敗後の解放、コールバック値の所属、arenaの再利用、ヘッダー取得後の参照と置換。安全性指摘はソース上の寿命追跡も再レビューする。

## Task 2：FFI／host境界のレイアウトと値管理

**Files:** `crates/thaw-llvm/src/hir_codegen/ffi_calls.rs`、`json_values.rs`、`dynamic_host/{callbacks,typed_calls,support}.rs`、host getter／setter生成箇所、`values/unions.rs`、`async_frames/`。
**Interface:** 型ごとのレイアウトと所有権を生成側・ランタイム側で一致させる。ABI変更なら関連宣言も同じ修正に含める。

- [ ] FFI object／tuple内の外部structを固定8バイトとして保存する経路を、実レイアウトに合わせる。
- [ ] host引数、getter／setter、結果の所有値を、全成功・失敗経路で解放する。既存release処理を共有する。
- [ ] Optional callbackはpresenceを確認してからペイロードを取り出し、省略時はundefinedを渡す。CallableFunctionも判定と登録側の対応型を一致させる。
- [ ] dynamic setter、typed host引数、async awaitの型メタデータ、JSON tuple／unionの既報指摘を対応表に従って修正する。
- [ ] ユーザー確認項目：省略／存在するcallback、複合型のhost往復、所有値の繰り返し呼び出し、失敗経路、await前後の型。

## Task 3：型に合った出力・安全な数値変換

**Files:** `crates/thaw-llvm/src/hir_codegen/{console,operators}.rs`、`crates/thaw-napi/src/napi/coercions.rs`。
**Interface:** 内部ポインタは型専用表示へ渡す。数値のToInt32／ToUint32契約を既存変換経路へ揃える。

- [ ] consoleのMap／Set／WeakMap／WeakSet／BytesをC文字列のfallbackから外し、型別表示を追加する。
- [ ] LLVMビット演算の直接fptosiを、非有限値の処理と32ビット剰余正規化を行う変換へ置き換える。
- [ ] NAPI int32の飽和castを修正し、隣接するuint32処理と契約を揃える。
- [ ] ユーザー確認項目：各コレクションのログ、負数・非有限値・32ビット範囲外の変換とシフト。表示処理が不正なポインタ型へ到達しないことをソースで確認する。

## Task 4：HIRの引数・レシーバー・プロパティ評価

**Files:** `crates/thaw-hir/src/lower/invocations/{arguments,calls,static_builtins,dynamic_values}.rs`、`instance_builtins/{conversion_methods,regex_methods,map_set_methods}.rs`、`expressions/lowering.rs`、オブジェクトlowering箇所。
**Interface:** 式はソース順に一度評価して既存のバインディング機構へ保存する。値が不使用でも副作用は保存する。

- [ ] Undefined型の引数式を省略扱いする前に評価する。default wrapper選択時に式を消さない。
- [ ] object methodのレシーバー、RegExp.toString、置換callbackの検索式を一度だけ評価する。
- [ ] String.rawの余剰引数、Promise tuple、structuredClone tuple、Map.getOrInsertComputedなどの引数評価を揃える。
- [ ] SuppressedError、重複オブジェクトキー、dynamic spread、any Map／Set mutationのバインディング順を修正する。
- [ ] 独自objectのmap等を名前だけで組み込みへ振り分けず、レシーバー型／関数プロパティで判断する。
- [ ] ユーザー確認項目：カウンターと評価順の記録で、通常／spread／default／tuple各経路の評価回数・順序を確認する。

## Task 5：JITの評価順をHIRと揃える

**Files:** `crates/thaw-cli/src/registry_integration/jit/{aggregates,callables,expressions}.rs`、`jit/returns.rs`、関連する既存JITテスト。
**Interface:** Task 4の「ソース順に一度評価」をJITでも守る。宣言フィールド順は保存先の配置にだけ使用する。

- [ ] object／union objectのプロパティをソース順に評価し、その後に宣言レイアウトへ配置する。
- [ ] logical &&の左辺副作用、mapのオペランド巻き上げ、helper引数の重複展開、unshift引数の逆順評価を修正する。
- [ ] ユーザー確認項目：同じ入力のHIR経路とJIT経路で、戻り値と副作用ログが一致することを確認する。

## Task 6：型推論・クラス・bridge生成

**Files:** `crates/thaw-hir/src/lower/{types,normalize}.rs`、`classes/{generic_classes,normalization}.rs`、`crates/thaw-cli/src/registry_integration/{class_methods,dynamic_declarations,shim_generation}.rs`、`crates/thaw-cli/src/module_graph.rs`、`crates/thaw-bridge/src/bridge/dts/{classes,exports,interfaces}.rs`。
**Interface:** parser／bridge／CLI／HIR間で公開名、型引数、引数省略、レシーバーと戻り型を一致させる。

- [ ] Voidを含む許可型のgeneric fingerprintを追加し、未知の型はpanicでなく診断にする。
- [ ] generic arrowの引数を本体推論スコープへ追加し、nested function型のスコープ漏れ、for-in／ofのゼロ回経路を修正する。
- [ ] break可能なwhile(true)を無条件終了として扱わず、暗黙のundefined返却を残す。
- [ ] class／methodの型引数shadow、内部identityとprivate fieldの列挙、tagged templateのsite identityを修正する。
- [ ] bridgeのoptional／rest、予約語export、union field、constructor overload、interface merge、intersection callable、namespace exportとCLI export aliasを対応表順に修正する。
- [ ] NAPI callback JsValueのadapter適用条件を生成可能条件と同一にする。
- [ ] ユーザー確認項目：生成宣言の公開名、各省略arity、void callbackのgeneric、arrow内の識別子、break後の戻り経路、独自クラスの列挙結果。

## Task 7：Promise・配列・Map／Set・JSONの意味論

**Files:** `crates/thaw-hir/src/lower/invocations/instance_builtins/`、`crates/thaw-runtime/src/runtime/{promises.rs,native_values/}`、`crates/thaw-llvm/src/hir_codegen/json_values.rs`、JSON／destructuringの既報箇所。
**Interface:** JSの戻り値、例外発生時点、変更中の走査、プロパティ制約を維持する。

- [ ] Promise.resolve／rejectの引数例外を同期例外として保ち、then(f,g)をthen(f).catch(g)へ変形しない。
- [ ] splice()のゼロ引数、Map／Set.forEachの変更中走査とthisArg、any clear()のundefined返却を修正する。
- [ ] numeric JSON setterにfreeze／preventExtensionsの共通制約を適用する。
- [ ] JSON.parseのEOF／数値境界、循環値処理、any destructuringのrenamed defaultを対応表に沿って修正する。
- [ ] ユーザー確認項目：then成功handlerが投げた例外、空splice、走査中追加・削除、clear戻り値、凍結済み値への書き込み、循環値、default式の評価。

## Task 8：文字列・正規表現・Buffer／Bytes

**Files:** `crates/thaw-runtime/src/runtime/native_values/{strings,regex,bytes}.rs`、`crates/thaw-arena/src/strings.rs`、HIR文字列・regex変換箇所、`crates/thaw-quickjs/src/quickjs/platform_globals/buffer_crypto.js`。
**Interface:** UTF-16／WTF-8表現を失わず、Buffer／TypedArrayの要素変換と範囲をAPI契約に合わせる。

- [ ] sort、join、trim、replace、normalizeなどのlone surrogate損失を共有変換経路で修正する。
- [ ] regex groupsのundefined、callback spread、sticky matchのlastIndex読取り・更新・リセットを修正する。
- [ ] split(undefined)、String(null)、Number(object)、数値property文字列化、fromCharCodeのcast前剰余を修正する。
- [ ] Uint8Array.set戻り値、setFromBase64のread、Buffer.fromのTypedArray要素変換・数値cast、checked boundsを修正する。
- [ ] Base64はpaddingで停止し、btoa／atobとBufferの異なる入力規則を混同しない。
- [ ] ユーザー確認項目：lone surrogateとNUL、undefined引数、sticky lastIndex、TypedArray種別、padding位置、要素・バイト境界。既存エンコーダを優先し独自実装を増やさない。

## Task 9：Date・Temporal・Intl

**Files:** `crates/thaw-hir/src/lower/invocations/temporal.rs`、`crates/thaw-runtime/src/runtime/native_values/{date,temporal}.rs`、`crates/thaw-quickjs/src/quickjs/intl.rs`、Number表示の既報箇所。
**Interface:** Date固有の0〜99年補正をTemporalへ流用しない。Durationのミリ秒とナノ秒を両方使う。

- [ ] Temporalのbag入力で年を補正せず、Duration.totalにナノ秒を含める。
- [ ] offset-only ZonedDateTimeの数字切り捨てをなくし、offsetの分範囲を確認する。Instantのoffset必須条件とUnicode解析の既報指摘も閉じる。
- [ ] Dateの日数計算前に安全な入力範囲を検査し、範囲外をInvalid Dateへ導く。
- [ ] Intl.PluralRulesの非有限値とNumber表示のundefined引数を対応する契約へ揃える。
- [ ] ユーザー確認項目：年0／99／100、1ナノ秒、正負offset、無効offset、巨大有限年、NaN／Infinity、undefined表示引数。

## Task 10：ファイル時刻・crypto

**Files:** `crates/thaw-quickjs/src/quickjs/filesystem.rs`、`platform_globals/buffer_crypto.js`、`crates/thaw-quickjs/src/quickjs/context.rs`。
**Interface:** ファイル時刻はepoch前後の符号を保存する。乱数と鍵長を黙って丸めない。

- [ ] SystemTime読取りでepoch以前の差分を負値として返し、utimes／lutimesで負値を保持する。
- [ ] randomIntの範囲検証、十分な乱数幅、棄却法を導入し剰余による偏りをなくす。
- [ ] PBKDF2の鍵長clampをなくし、指定長または明示的な入力エラーを返す。
- [ ] ユーザー確認項目：epoch前後の時刻往復、乱数範囲上限と生成方式、PBKDF2の境界鍵長。乱数の正しさは頻度検査だけで判断せずアルゴリズムもレビューする。

## Task 11：非同期コンテキスト・UDP・HTTPクライアント

**Files:** `crates/thaw-registry/src/registry/builtins/system/async_hooks.rs`、`registry/builtins/network.rs`、`crates/thaw-quickjs/src/quickjs/networking.rs`、`crates/thaw-runtime/src/runtime/http.rs`。
**Interface:** 呼び出し元のストアは同期的に復元し、非同期継続は作成時のコンテキストを使う。UDP受信はcloseまで継続する。

- [ ] ALSのPromise.finallyによる遅延復元をやめ、既存非同期実行基盤で継続ごとのコンテキストを保持する。単なる即時復元だけでは非同期内の保持を解決したことにしない。
- [ ] UDPは1回受信で終了せず、イベントループへ受信継続を登録し、closeで解除する。同期再帰やbusy loopにしない。
- [ ] HTTP URLのauthorityをquery／fragment境界でも区切り、fragment-only redirectで元queryを保持する。
- [ ] 1xx暫定応答の後に最終応答を読み進める。Task 1のヘッダー寿命変更と整合する。
- [ ] ユーザー確認項目：並行ALS、nested run／exit、Promiseを返すrun直後、UDP複数受信とclose、パスなしquery、fragment redirect、暫定＋最終HTTP応答。

## Task 12：レジストリ・バンドル・Worker

**Files:** `crates/thaw-registry/src/registry/bundle/{resolution,render,workers}.rs`、`registry/module_transform.rs`、`registry/install/package.rs`、`registry/install/metadata.rs`。
**Interface:** exportsがある場合は公開された経路のみ解決する。bundle固有のWorker状態を共有グローバルで上書きしない。

- [ ] exports不一致時の実ファイルfallbackを止め、exports不在時の既存解決は維持する。
- [ ] 入れ子dynamic importの重複範囲を扱える書き換えにする。文字列位置のguardだけで内側importを取り残さない。
- [ ] wildcard captureでprefix＋suffixの長さを検査する。
- [ ] CJS factory失敗時のcacheを撤回し、Workerが作成元bundleの解決関数・sourceを保持するようにする。
- [ ] registry再登録時の実行ファイルとlock metadataの更新を対応表に沿って修正する。
- [ ] ユーザー確認項目：exportsの公開／非公開／条件分岐、入れ子import、重なるwildcard、失敗後の再require、複数bundle後のWorker、再登録した成果物。

## Task 13：CLI・アドオン保存先・サンプル・test互換

**Files:** `crates/thaw-cli/src/registry_integration/shim_generation.rs`、`crates/thaw-cli/src/{dev,build}.rs`、QuickJSのCLI起動処理、`crates/thaw-parser/src/lib.rs`、`crates/thaw-registry/src/registry/builtins/testing.rs`、`examples/hono-react-prisma-board/server.ts`。
**Interface:** パッケージの識別を失わず保存先を一意にし、配信時はasset encodingと本文型を一致させる。

- [ ] 外部addonの保存先を衝突しない形式へ変える。JS識別子用sanitizeをfilesystem識別に流用しない。
- [ ] entry親外の依存watch、裸のfoo.js起動、parserのfilename診断を既報条件で修正する。
- [ ] node:test planのassertion件数を実際の呼び出しへ連動させる。
- [ ] 条件付きサンプル指摘：hex化された非UTF8 assetをbytesへ戻して配信する。既存asset配信ヘルパーを優先する。
- [ ] ユーザー確認項目：sanitize結果が同じ2パッケージ、外部依存更新、相対ファイル起動、診断filename、plan件数、画像asset本文のバイト一致。

## Task 14：全体の完了判定

**Interface:** Task 0の対応表へ、各タスクの修正差分・再レビュー根拠・ユーザー確認結果を戻す。

- [ ] 確定指摘をすべて「修正済み＋確認済み」「既存変更で解決済み＋根拠あり」「ユーザーが明示的に延期」のいずれかへ対応付ける。未割当と理由なしの延期を残さない。
- [ ] 型／ABIを変えたタスクは宣言・呼び出し・ランタイム全体を再レビューする。
- [ ] HIRとJIT双方、NAPIとQuickJS双方に同じ原因が残っていないことを横断確認する。
- [ ] ユーザーから各修正単位の確認結果を受け取り、失敗項目は該当タスクへ戻す。こちらでテストを実行しない。
- [ ] 未修正・条件付き・検証範囲外を最終報告へ明記する。静的確認だけをもって実行確認済みとは報告しない。

## 自己点検

- 安全性、評価順、型生成、標準API、レジストリ、CLI／サンプルの各領域をタスクへ配置した。
- 全履歴の指摘ID別網羅性はTask 0で確定する。この計画だけを完全な指摘台帳とは扱わない。
- テスト実行をユーザーに委ねる指示を、全タスクの完了条件へ反映した。
- 実装の見積もりはTask 0完了後に出す。静的指摘数だけから工数・完了日を断定しない。
