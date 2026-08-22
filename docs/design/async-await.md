# async/await 設計ドキュメント

- ステータス: 設計のみ（未実装）
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
- `CoroSplit` 等の最適化パスを走らせる最小のパスパイプライン構築方法
  （新しい PassManager API を inkwell 経由でどこまで叩けるか）。
- エラー伝播（`try/catch` は現状 [同一関数内限定](../../crates/thaw-llvm/src/hir_codegen.rs)
  -- コルーチンがサスペンドを挟んで再開された後の catch は「同一関数」の
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
3. `Promise.all` 等の並行実行が実際に要求される段階になったら、
   4章の設計を土台に V2 に着手する。
