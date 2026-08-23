# async/await 設計ドキュメント

- ステータス: V1実装済み。V2はPromise ABIと単一スレッド継続キューまで実装済み、
  タイマーイベント源、トップレベル駆動ループ、`await sleep(ms)`を持つ
  async mainのramp/resumeフレーム分割まで実装済み。
  トップレベルおよびネストした制御フローのローカルframe slot、一般async関数、
  async try/catchの例外伝播、fdのI/O readiness登録とpollイベントループ統合も
  実装済み。非ブロッキングHTTP状態機械とasync fetch接続も実装済み
- 前提: [thaw-hir](../../crates/thaw-hir), [thaw-llvm/hir_codegen](../../crates/thaw-llvm/src/hir_codegen.rs), [thaw-runtime](../../crates/thaw-runtime) の現状（Phase 0〜2一部）を前提にする

## 1. 目的とスコープ

トップレベル設計ドキュメントの技術スタック節では非同期処理を「LLVM コルーチン
intrinsics（`llvm.coro.*`）でコンパイルする」と決めている。これは長期的に正しい
方向だが、本格的な `llvm.coro.*` ローワリングは以下の理由で単体として大きい：

- コルーチンフレームのレイアウト計算・確保（`coro.id` / `coro.begin` /
  `coro.size` / `coro.alloc` / `coro.free`）
- サスペンド地点ごとの再開ステートマシン（`coro.suspend` / `coro.resume` /
  スイッチベース分岐）
- 実際に「待てるもの」（`fetch` や Promise を返す npm 関数）がまだ存在しない
  ため、機構だけ作っても検証しにくい

そのため本ドキュメントは **2段階** の設計を提示する。

- **V1（実装推奨・小工数）**: `async`/`await` の構文と型だけを先に通し、
  実行時には同期呼び出しにコンパイルする。コルーチンフレームは作らない。
- **V2（本実装・大工数）**: 実際に `llvm.coro.*` を使い、単一スレッドの
  ノンブロッキングイベントループの上で真にサスペンド/レジュームする。

V1 は「Lambda ハンドラの中で `await fetch(...)` を数回、順番に呼ぶ」という
Thaw の主要ユースケース（Web サーバーのような大量同時 I/O ではない）を
100% カバーできる。V2 が要るのは `Promise.all` のような**並行**待機が
必要になったときで、それは Thaw の当面のターゲット（1リクエスト1プロセスの
Lambda 実行環境）では優先度が低い。

## 2. 制約条件

- **GC なし・アリーナベース**（メモリモデルは3.2章参照）。コルーチンフレームを
  ヒープ確保するなら arena 経由にする。スレッドやOSレベルのスタック切り替え
  （ucontext 等）は使わない -- 太いランタイムを持ち込むことになり、コールド
  スタート最速というテーゼに反する。
- **tokio/hyper を実行時ランタイムの中核には使わない**（[thaw-runtime](../../crates/thaw-runtime/src/lib.rs)
  で Lambda Runtime API クライアントを素の `TcpStream` で書いたのと同じ理由）。
- HIR の `HirType::Promise(Box<HirType>)` と `HirExpr::Await` は既に
  [thaw-hir/src/lib.rs](../../crates/thaw-hir/src/lib.rs) にスケルトンとして
  存在するが、まだどの lowering パスからも構築されない（未使用）。
- 既存の関数コンパイルモデル（`declare_function` → `compile_function_body`）
  や、値表現（f64 / bool / 文字列・配列・オブジェクトはポインタ）を大きく
  崩さずに拡張できることが望ましい。

## 3. V1: 同期脱糖（推奨する最初の実装）

### 3.1 考え方

Lambda の実行モデルでは、1プロセスは同時に1つの呼び出ししか処理しない
（[thaw-runtime](../../crates/thaw-runtime/src/lib.rs) のループも1件ずつ）。
この前提の下では、`async function` 内の `await` は「本当に他の処理と
インターリーブする必要がある一時停止」ではなく、「結果が返るまで待つ
同期呼び出し」として振る舞わせても、観測できる違いがない。

つまり:

```ts
async function fetchStage(): Promise<string> {
  const v = await someAsyncNativeCall();
  return v;
}
```

は実行時には次と等価にコンパイルする:

```
fn fetchStage() -> Str {
    let v = someAsyncNativeCall_blocking();  // 呼び出し即ブロック
    return v;
}
```

`Promise<T>` はコンパイル後には現れない。`async function` はただの
「戻り値の型が `Promise<T>` と書かれた関数」として扱い、実際の戻り値は
`T` そのもの（コルーチンフレームもステートマシンも生成しない）。

### 3.2 HIR 拡張

- `HirFunction` に `is_async: bool` を追加。
- 型チェック（lowering 時点の軽い整合性チェック）: `async function` の
  宣言上の戻り値は `Promise<T>` でなければならない。lowering 後の
  `HirFunction.ret` は `T` を直接格納する（`Promise` でラップしない） --
  これは「V1 ではコンパイル後に Promise 型が存在しない」という設計の帰結。
- `HirExpr::Await(Box<HirExpr>)` は既存のものをそのまま使う。lowering 時に
  `await expr` は「`expr` を評価するだけ」（`HirExpr::Await` ノード自体を
  作らず `expr` をそのまま返す）か、後述のデバッグ/将来拡張のために
  `Await` ノードを作って codegen 側で恒等変換するかは実装コストがほぼ同じ。
  **将来 V2 に差し替えるときの diff を小さくするため、`Await` ノードは
  残す** ことを推奨（lowering は `HirExpr::Await(Box::new(lower_expr(arg)?))`
  を作り、V1 の codegen は「中身を評価してそのまま返す」だけの恒等関数として
  扱う）。

### 3.3 Codegen（V1）

`hir_codegen.rs` 側の変更は小さい:

- `compile_expr` に `HirExpr::Await(inner) => self.compile_expr(inner)` を
  追加するだけ（中身を評価した値をそのまま返す = 恒等変換）。
- `async function` の関数シグネチャ生成（`declare_function`）は非 async
  関数と全く同じパスを通る（戻り値は `T` であって `Promise<T>` ではない
  ため、`basic_type` 側で `Promise` を特別扱いする必要すらない）。
- `HirType::Promise` は codegen に到達しない（lowering で剥がされているため）
  -- `basic_type` の「未対応」エラーパスに来たら lowering のバグ。

### 3.4 `await fetch(...)` はどうなるか

V1 の時点では `fetch` 自体がまだ存在しない（`thaw-bridge`/`std` 未実装、
Phase 3 以降の課題）。`fetch` を先に実装するなら、V1 の間は
「ブロッキング HTTP クライアント（[thaw-runtime](../../crates/thaw-runtime/src/lib.rs)
と同じ思想の、素の `TcpStream` + 必要なら TLS ライブラリ1個だけ）を呼んで
結果を同期的に返す」関数として実装すればよく、`await` はその呼び出しに
対して何もしない（3.3節の恒等変換）。

### 3.5 制限事項（V1で明示的に諦めるもの）

- `Promise.all` / `Promise.race` のような**並行**待機は意味を持たない
  （すべて逐次実行になる）。呼んだらエラーにするか、逐次実行にフォール
  バックするかは実装時に選ぶ（後者は「動くが並行にならない」ため、
  エラーにして誤解を防ぐ方が誠実）。
- コールバックベースの Promise（`new Promise((resolve, reject) => ...)`）は
  V1 では未対応。QuickJS-NG 側の Promise 解決を Thaw 側から同期的に
  「待つ」ブリッジ関数があれば技術的には可能だが、Bridge 設計
  （thaw-bridge, Phase 3〜4）が先。

## 4. V2: 本物の LLVM コルーチン

V1 で並行待機や真の一時停止が必要になった時点で着手する。ここではアーキ
テクチャレベルの設計と、詰めるべき論点を記す（実装はしない）。

### 4.1 LLVM コルーチン intrinsics の概要

LLVM のコルーチンサポート（[Coroutines in LLVM](https://llvm.org/docs/Coroutines.html)）
は次の intrinsics 群からなる:

| intrinsic | 役割 |
|---|---|
| `llvm.coro.id` | コルーチンの識別子を生成。フレームのメモリレイアウトを LLVM の coroutine-splitting パスが決定するためのマーカー |
| `llvm.coro.size` / `llvm.coro.align` | フレームサイズ・アラインメントを実行時に取得 |
| `llvm.coro.alloc` / `llvm.coro.free` | フレーム確保が必要かの判定・実際の確保/解放を呼び出す側の任意のアロケータに委譲するためのフック |
| `llvm.coro.begin` | フレームの初期化、以降の処理で使うコルーチンハンドルを返す |
| `llvm.coro.suspend` | サスペンド地点。戻り値で「サスペンドした/コルーチン終了/破棄された」を分岐 |
| `llvm.coro.resume` / `llvm.coro.destroy` | 外部から再開/破棄する |
| `llvm.coro.end` | コルーチン関数本体の終端 |

重要なのは `coro.alloc`/`coro.free` が **呼び出し側の任意のアロケータ**
にフックできる設計になっていること。これは Thaw の GC なし・アリーナ
モデルとの相性が良い: `coro.alloc` の実体を `thaw_arena_alloc` 呼び出しに
差し替えれば、コルーチンフレームもリクエストスコープのアリーナから
確保され、リクエスト終了時の一括解放（`thaw_arena_reset`）でまとめて
片付く。GC 由来の一時停止は発生しない。

### 4.2 HIR 拡張（V1 からの差分）

- `HirFunction.is_async` はそのまま使う。
- `HirType::Promise(Box<HirType>)` はコンパイル後も残る型になる
  （V1 では消えていたが、V2 では `async function` の戻り値は本当に
  「まだ完了していないかもしれない計算」を表すハンドルになる）。
- `HirExpr::Await` はもう恒等変換ではなく、実際のサスペンド地点になる。

### 4.3 コルーチン関数のコンパイル形状

1つの `async function` は次の3つの LLVM 関数に分解される（Clang が
C++ コルーチンをこう分解するのと同じパターン）:

- **ramp function**: 呼び出し側から見える入口。`coro.id`/`coro.begin` で
  フレームを確保し、初回の実行を進めて最初の `coro.suspend` まで走らせ、
  呼び出し元には `Promise` ハンドル（フレームへのポインタ + 状態）を
  即座に返す。
- **resume function**: `await` していた値の準備ができたときに呼ばれる。
  フレームに保存された「次に実行する地点」に `switch` 命令で分岐し、
  続きを実行する。
- **destroy function**: キャンセル/エラー時にフレームを解放するパス。

これらは LLVM の `CoroSplit` パスが `coro.suspend` の数から自動生成する
ため、Thaw 側が手で3つ書く必要はない -- Thaw のコード生成は「1つの
関数本体の中に `coro.*` intrinsics を正しい順序で埋め込む」ところまでで、
分割は LLVM 側のパスに任せる。**ただし** これは inkwell/thaw-llvm 側で
最適化パスパイプライン（現状の `write_object_file` は最適化パスを一切
走らせていない、`OptimizationLevel::Default` は `TargetMachine` の
命令選択チューニングだけで IR レベルのパスは含まれない）を組む必要が
あることを意味する。つまり V2 は「コルーチン IR を出力する」だけでなく
「`CoroSplit`/`CoroCleanup` を含む最小限のパスパイプラインを自前で
呼び出す」作業も追加で必要になる。

### 4.4 サスペンド地点とレジューム値の受け渡し

`await expr` の変換（疑似コード、C++コルーチンの `co_await` 変換と同型）:

```
%promise = <expr の評価>                  ; Promise ハンドル
call thaw_promise_subscribe(%promise, @this_coro_resume_fn, %frame)
%suspend_result = call i8 @llvm.coro.suspend(...)
switch %suspend_result, label %resume [ i8 0, label %resume
                                         i8 1, label %cleanup ]
resume:
  %value = <フレームに保存された、subscribe が書き込んだ結果値>
  ; ここから続きの処理
```

`thaw_promise_subscribe` が本設計の要になる: 「このコルーチンを、
`%promise` が解決したら `%this_coro_resume_fn` で再開してくれ」と
イベントループ（4.5節）に登録する、Thaw ランタイム側の関数。

### 4.5 イベントループ

tokio は使わない。Lambda の1リクエスト1プロセスモデルでは、必要なのは
「今アウトスタンディングな I/O のうちどれかが準備できるまで待ち、
準備できたコルーチンを resume する」というだけの単純なループで足りる:

- Linux では `epoll`（`epoll_create1`/`epoll_ctl`/`epoll_wait` を
  `libc` 経由で直接呼ぶ。crate `libc` は薄い FFI 宣言集めであり
  ランタイムではないため、「重いランタイムを持ち込まない」方針と矛盾しない）。
- 待機中のコルーチンハンドルを `fd -> resume関数ポインタ` のテーブルで
  管理する、数十行程度の小さなイベントループを thaw-runtime に追加する。
- `thaw_runtime_run`（[thaw-runtime](../../crates/thaw-runtime/src/lib.rs)）
  の1リクエスト処理は、「handler を呼ぶ」→「返ってきたのが未完了の
  Promise ならイベントループを回して完了させる」→「レスポンスを POST する」
  という形に変わる。

### 4.6 Bridge（QuickJS-NG）との接続契約

設計ドキュメント3.1節の通り、Promise を返す npm パッケージ関数は
「Bridge 側でコールバック → コルーチン変換を行う」。V2 の
`thaw_promise_subscribe` は、QuickJS 側の Promise の `.then()` に
登録したネイティブコールバックから叩かれることを想定した、以下のような
安定 ABI にする必要がある（詳細は thaw-bridge 設計時に確定）:

```
// Thaw コード生成側が呼ぶ
extern "C" fn thaw_promise_subscribe(
    promise: *mut ThawPromise,
    resume_fn: extern "C" fn(*mut u8 /* frame */, *const u8 /* result */),
    frame: *mut u8,
);

// QuickJS 側の .then() コールバックから呼ばれる、Bridge 側の実装
extern "C" fn thaw_promise_resolve(promise: *mut ThawPromise, result: *const u8);
```

このABIは `thaw-runtime` に実装済み。`resolve`/解決済みPromiseへの`subscribe`
は継続を直接再入呼び出しせず、FIFOのランキューへ積む。
`thaw_runtime_poll_one` / `thaw_runtime_run_until_idle` が同じスレッド上で継続を
実行するため、今後QuickJSジョブキューとI/O readinessを交互にpollできる。
Promise状態はpending=0、fulfilled=1、rejected=2で、`thaw_promise_reject`も
resolveと同じFIFO継続キューを一度だけ起動する。settled結果ポインタが通常値か
エラーかは`thaw_promise_state`で判別する。
async状態内のthrowは完了Promiseをrejectする。awaitのresume入口は待機Promiseを
破棄する前に状態を確認し、rejectedなら型付き結果slotを読まず、同じエラーポインタで
呼び出し元の完了Promiseもrejectする。これによりcatchがない多段awaitで拒否が伝播する。
同一async関数のtry本体にある直接のthrowはcatch変数frame slotへエラーを保存し、
try guardを無効化してcatch guardを有効化する。catch本体自体もawaitできる。
try内の各await後resume状態にはcatchメタデータを付ける。子Promiseがrejectedなら
型付き結果を読まず、catch変数slotへエラーを保存してtry guardを落とし、catch guardを
立てて同じ状態を再開する。try外のrejectionは従来どおり上位Promiseへ伝播する。
try/finallyだけの場合はloweringが生成した合成catchを同じ仕組みで有効化し、注入済み
finalizerを実行してから完了Promiseを再rejectする。catch/finallyではcatchが正常完了
した後に通常経路のfinalizerを一度だけ実行する。
catch内のrethrowはcatch guard付きのreject分岐へ変換する。catch内で再度awaitした後でも
新しいエラーポインタを上位Promiseへ伝播し、finallyがある場合はloweringがrethrow直前へ
注入したfinalizerを一度だけ実行する。
ネストtryでは内側tryのresume状態に内側catch、内側catch内のresume状態に外側catchの
メタデータを付ける。これにより各rejectionは実行位置から最も近いcatchへ送られる。
`thaw_sleep_ms` はワーカースレッドを作らずタイマーPromiseを登録し、
`thaw_runtime_run_until_resolved` がランキューとタイマーを駆動してトップレベルの
Promise完了まで待つ。これによりLLVM coroutine loweringを接続する実イベント源が
用意された。

`thaw_runtime_wait_fd(fd, interests)` はreadable/writable待機をPromiseとして登録する。
イベントループはタイマーの次回期限をtimeoutにして`poll(2)`を呼び、fd readinessと
タイマーを同じスレッドで駆動する。Promise破棄時にはfd登録も解除され、無効fdは
rejectionになる。pollへブロックする直前には対象Promiseを再確認し、同じ反復で
満了したタイマーを見落として別fdを無期限待機しない。

`thaw_http_get_async`は平文HTTP GETをconnect/write/readの状態機械へ分解し、各段階を
fd readiness Promiseで再開する。`await fetch(...)` のframe分割時だけこのABIを使い、
awaitされない既存fetchは互換性のため同期ABIを維持する。HTTP全体期限はfd待機期限へ
引き継がれ、接続エラー、peer切断、不正応答、HTTPエラー、タイムアウトはcompletion
Promiseのrejectionになる。レスポンス解析はreadごとに増分実行し、`Content-Length`分の
本文またはchunked終端を検出した時点でkeep-alive接続の切断を待たず完了する。
chunk extensionとtrailerを含む`Transfer-Encoding: chunked`をデコードして、呼び出し側には
連結済み本文を返す。`https://`はrustlsのハンドシェイク・暗号化write/readを同じfd readiness
状態機械へ統合し、WebPKIルート、SNI、証明書名・署名・有効性を検証する。TLSエラーと
ハンドシェイク中の期限切れもPromise rejectionになる。301/302/303/307/308はLocationを
絶対URL、scheme-relative、絶対パス、相対パスとして解決し、同じcompletion Promise、
TLS設定、全体期限を維持したまま状態機械を再接続する。HTTPからHTTPSへの遷移にも対応し、
10回を超えるリダイレクトはrejectionにする。DNS解決は専用ワーカースレッドで実行し、
非ブロッキングpipeのreadable通知を既存のfdイベントループへ接続する。初回接続と
リダイレクト後の再接続は同じ経路を使い、名前解決中もタイマーと他のI/Oを駆動できる。
DNS時間もHTTP全体期限に含み、解決失敗や空のアドレス集合はPromise rejectionになる。

async関数内のトップレベル`HirExpr::Await(Call("sleep", ...))` はframe分割へ
接続済み。関数引数はrampで型付きframe slotへコピーされ、resume後も同じ変数として
参照できる。ramp関数は
完了Promiseを即座に返し、独立した内部resume関数がフレームの状態番号をswitchして
複数awaitの続きを実行する。フレームは完了Promise、状態番号、現在待機中のPromiseを
保持し、待機Promiseはresume時に破棄される。C mainが完了Promiseまでイベントループを
駆動する。関数トップレベルの`let`/`const`は型ごとの固定frame slotへ保存され、
resume関数の全状態が同じslotを変数表として使うため、awaitをまたぐ読み書きができる。
`Promise<void>`の明示`return;`は完了Promiseを解決して後続状態を実行せず終了する。
値を返す関数ではframe内の専用結果slotへreturn値を保存し、そのslotへのポインタで
完了Promiseを解決する。`const value = await compute()` の形では、呼び出し元のresume
関数が継続引数の結果ポインタから型付き値を読み、呼び出し元frameのlocal slotへ保存する。
どの関数がframe ABIを使うかは、`sleep`を起点にawait呼び出し関係を固定点までたどって
宣言前に決定するため、V1の同期脱糖だけを使う既存async関数のABIは変わらない。
式の途中にあるawaitは左から順に抽出し、型付きの一時frame slotとresume状態へ展開する。
このため、1つの式に複数のawaitがある場合も評価順を保ったまま中断・再開できる。
ifの条件式にあるawaitも同じ方法で抽出し、resume後に分岐を評価する。then/else本体は
分岐guard付きの状態列へ展開し、選ばれなかった側ではPromiseを生成せず次状態へ進む。
分岐内のローカル宣言は関数のframe slotへ収集され、awaitをまたいで保持される。
分岐内のreturnは完了Promiseを解決してresume関数を終了し、throwは現在の例外handlerへ
遷移する（handlerがなければ完了Promiseをrejectする）。while本体のawaitは条件状態、本体のguard付き状態、
条件状態へ戻る後退エッジへ展開する。ループ条件のawaitも条件状態列の一部になり、
反復ごとにPromiseを作り直して結果を再評価する。breakは本体guardと後退edge guardを
落として合流状態へ進み、continueは本体guardだけを落として条件状態へ戻る。
ループ内のローカル宣言・return・例外も同じguard付き展開を使う。

ネストしたifは親guardから子のthen/else guardを生成する。親が非選択なら子の条件式も
評価せず、任意段数のif本体にあるawaitをguard付き状態へ展開できる。async while内の
ネストifからのbreak/continueは外側ループの本体guard・後退edge guardへ伝播する。
ネストifの条件式にあるawaitも親guard付き状態へ展開するため、親が非選択なら条件の
Promise自体を生成しない。条件とthen/else本体の両方にawaitがある場合にも対応する。
ネストしたwhileは親guardからloop-enabled guardを作り、内側ループ専用の条件状態、
本体guard、後退edgeを再帰的に生成する。親が非選択なら内側の条件Promiseも作らない。
例外を投げないtry/finallyはlowering済みHIRのfinally複製を利用し、try本体を通常の
async状態列へ展開する。正常完了ではtry後のfinalizer、returnではreturn直前へ注入済みの
finalizerが実行される。throw/rejectionを伴うtry/catch/finallyも、await状態ごとの
rejection handlerとtry/catch guardで処理する。try/catchは任意の深さにネストでき、
if/while内のtry/catchとtry/catch内のif/whileの双方を再帰的に展開する。
ネストした宣言とcatch引数はlowering時に字句スコープを追跡し、同名の外側bindingを
シャドーイングする場合は一意なHIR名へ変換する。それぞれ独立したframe slotを使うため、
awaitをまたいでも内外の値が混同されない。

QuickJS-NG 自体はシングルスレッド・ノンブロッキングな評価モデルなので、
Thaw 側のイベントループと QuickJS のジョブキュー（`JS_ExecutePendingJob`）
を同じスレッド上でラウンドロビンさせる必要がある -- これは thaw-bridge
設計時の重要な論点として引き継ぐ。

### 4.7 未解決の論点（V2着手時に決めること）

- フレームサイズが `coro.size` でしか実行時に分からない以上、
  `thaw_arena_alloc` 呼び出しをどのタイミングで挿入するか（`coro.alloc`
  intrinsic のカスタムロワリングをどう inkwell/LLVM C API から行うか）。
  inkwell が `coro.*` intrinsics をどこまで安全にラップしているか未調査
  -- 最悪 raw な `LLVMAddFunction`/`LLVMBuildCall2` で intrinsic 宣言を
  手動構築する必要がある。
- `CoroSplit` 等の最適化パスを走らせる最小パイプラインは実装済み。
  `write_object_file` が新Pass Manager APIで
  `coro-early,coro-split,coro-cleanup` を実行する。コルーチンを含まない
  V1プログラムではno-opとなる。
- エラー伝播（同期関数間の `try/catch` は保留例外スロット方式で実装済みだが、
  コルーチンがサスペンドを挟んで再開された後の catch は「同一関数」の
  定義が曖昧になる。resume function 内で再度 try/catch の分岐を再構築
  する必要があり、Phase 1 の try/catch 実装を素直に拡張できない可能性が高い）。
- キャンセル（Lambda のタイムアウトで実行中のコルーチンを中断する場合、
  `coro.destroy` 経路とアリーナ解放のタイミング）。

## 5. 移行パス

V1 → V2 で HIR 側のインターフェース（`HirExpr::Await`, `HirFunction.is_async`,
`HirType::Promise`）は変えずに済むよう設計してある。変わるのは
`hir_codegen.rs` の `Await`/async 関数まわりの実装だけ（恒等変換 →
本物のサスペンド生成）。呼び出し側（lowering, thaw-cli）に波及する変更は
想定していない。

## 6. まとめ: 実装順の提案

1. **V1 をまず実装する**（本ドキュメントの3章）。工数は小さく、
   Lambda ハンドラでの逐次 `await` はこれで完全にカバーできる。
2. `fetch`/`std` の実装と合わせて、V1 の「ブロッキング呼び出しに
   脱糖する」対象を増やしていく。
3. `Promise.all`のhomogeneous number fan-inは7章の形で実装済み。次は同じ
   schedulerをheterogeneous tupleと他のPromise結合子へ一般化する。

## 7. Promise.all fan-in

`Promise.all([p0, p1, ...])`は現在、homogeneousな`Promise<number>`のarray literalに
限定してHIRへ取り込む。lowering後は`Call("Promise.all", children)`となり、戻り型は
`Promise<number[]>`である。空配列は許可し、非Promiseまたはnumber以外の要素は要素番号を
含むコンパイルエラーにする。

LLVMは全child callを先に評価し、handle配列を`thaw_promise_all_f64`へ渡す。runtimeは全handleへ
同時にsubscribeし、入力indexのarena slotへ結果をコピーするため、完了順が変わっても結果順は
変わらない。結果は`[i64 len][f64...]`へのpointerを格納したtyped result slotとして親Promiseへ
渡し、通常のframe resume ABIから読み取る。

最初に観測したreject pointerを保持し、それ以後の成功値は無視する。未完了childをLambda request
境界の外へ残さないため、全child callbackを回収してhandleを破棄した時点で親をrejectする。
runtime testは空配列、順序、最初のerror、3本の80ms timerが直列化されないことを検査する。
LLVM E2Eはユーザー定義async関数、`if`、`while`、`try/catch`との合成を単一実行ファイルで検査する。
