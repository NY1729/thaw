# Structured Clone Graph Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `structuredClone` が循環参照と共有参照を正しく複製し、深いグラフでも再帰オーバーフローせず、Host getter／列挙失敗時に部分cloneやleaseを残さないようにする。

**Architecture:** `Value::deep_clone_mode`の再帰的なtree copyを、identity memo・明示worklist・遅延commitを持つgraph clone engineへ置き換える。clone先の空shellをmemoへ先に登録し、全nodeの読取りとHost操作が成功してから辺を一括commitすることで、cycleを保持しつつfailure atomicityを確保する。公開TypeScript APIとC ABIは維持し、`__thaw_json_clone`から既存の`thaw_json_structured_clone(ptr) -> ptr`を呼び続ける。

**Tech Stack:** Rust、`Rc<SharedValue<_>>`、`IndexMap`、Thaw HIR、LLVM/inkwell、QuickJS Host bridge、Cargo test、Node compatibility harness

**Spec:** `thaw-execution-runtime-audit-2026-10-19.md` の R6「循環`structuredClone`でオーバーフローする」

**Intended repository path:** `docs/superpowers/plans/2026-10-19-structured-clone-graph-identity.md`

## Global Constraints

- 実装ブランチ名は `fix/structured-clone-graph-identity` とする。
- 基点はGitHub `main`の `6da9cf201f1d42a3682ba806b6f22e11524ce37a` とする。
- 現在の `/workspace/thaw` は別修正の未コミット変更を含むため、そこへ書き込まない。
- 実装時は外部のisolated worktree、推奨パス `/tmp/thaw-worktrees/fix-structured-clone-graph-identity` を使う。
- TypeScript側の`structuredClone(value)`シグネチャと、C ABI `thaw_json_clone(*const Value) -> *mut Value`／`thaw_json_structured_clone(*const Value) -> *mut Value`は変更しない。
- 新規dependencyは追加しない。
- clone計算量は、到達可能なcontainer数を`V`、辺数を`E`として時間`O(V + E)`、補助空間`O(V + E)`とする。
- production codeより先に、各behaviorを固定する失敗テストを追加してREDを確認する。
- array hole、WTF-8 key、BigInt、wrapper brand、graph-plain marker、既存N-API lease共有規則を維持する。
- `native_links`、prototype、frozen/sealed/extensible stateは現実装もcloneしていないため、この修正では新たにcloneしない。
- transfer list、Function、Date、RegExp、Map/Setなど非plain Host objectの対応範囲は広げない。現状どおり非plain Host leaseを共有する。
- typed HIRの`Array`／`Tuple`／固定`Object`／native `Map`／`Set` clone経路は変更しない。今回の主対象は`HirType::Json`のruntime graphである。
- Host errorはclone直後に消費し、失敗結果を`thaw_json_destroy`してから既存例外経路へ送る。

## Review Focus

- **自己循環と相互循環:** clone先の辺は必ずclone先shellを指し、元graphへ戻らないこと。Task 1とTask 4で固定する。
- **共有部分グラフ:** 同一source identityはclone内でも一つだけ生成され、元identityとは分離されること。Task 1とTask 4で固定する。
- **深い非循環graph:** 4,096段以上でもRust call stackに依存せずcloneできること。Task 1で固定する。
- **plain Host graph:** 同じQuickJS handleを複数回見ても一度だけmaterializeし、cycleとaliasを維持すること。Task 2で固定する。
- **途中失敗とreentrant cleanup:** getter／enumeration失敗時に部分cycle、Host lease、N-API lease、stale `HOST_ERROR`を残さないこと。Task 2とTask 3で固定する。

---

## File Structure

- **Create:** `crates/thaw-std/src/json/structured_clone.rs`  
  identity-aware graph clone engine、worklist、staged fill、clone metadata処理、同moduleのunit testsを置く。
- **Modify:** `crates/thaw-std/src/json.rs`  
  `mod structured_clone;`を宣言し、既存`deep_clone`／`deep_clone_mode`とexported FFI関数を新engineへ委譲する。array-hole snapshot/install用の小さな内部helperを追加する。
- **Modify:** `crates/thaw-llvm/src/hir_codegen/invocations/json_calls.rs`  
  `__thaw_json_clone`の戻り値について、直後に`compile_check_json_host_error(..., Some("thaw_json_destroy"))`を実行する。
- **Modify:** `crates/thaw-llvm/src/hir_codegen/tests/values/dynamic_and_json.rs`  
  cycle、alias、independence、plain Host graph、getter failure、LLVM IRのerror-check順を検証する。
- **Modify:** `tests/builtins-compat.json`  
  Nodeとの差分を継続検証する`structuredClone.cycle-and-shared-identity`を追加する。
- **No change:** `crates/thaw-hir/src/lower/invocations/structured_clone.rs`  
  `HirType::Json -> __thaw_json_clone`のdispatchは正しく、runtime実装だけを差し替える。

---

### Task 1: Native Json graph cloneをcycle-safeかつstack-safeにする

**Files:**
- Create: `crates/thaw-std/src/json/structured_clone.rs`
- Modify: `crates/thaw-std/src/json.rs`
- Test: `crates/thaw-std/src/json/structured_clone.rs` の `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes:
  - `Value`
  - `SharedArray`
  - `SharedObject`
  - `object_identity_key(&Value) -> Option<usize>`
  - `shared_array_ref`／`shared_array_ref_mut`
  - `shared_object_ref`／`shared_object_ref_mut`
  - `ARRAY_HOLES`
  - `is_branded_wrapper`
  - `brand_internal_value`
  - `is_graph_plain_object`
  - `mark_graph_plain_object`
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub(super) enum CloneMode {
      Snapshot,
      Structured,
  }

  pub(super) fn clone_value(
      source: &Value,
      mode: CloneMode,
  ) -> Result<Value, ()>;
  ```
- Error convention: `Err(())`は`HOST_ERROR`へ原因が記録済みであることを意味する。Task 1のnative-only経路は`Err`を返さない。
- Internal types:
  ```rust
  struct GraphCloner {
      mode: CloneMode,
      memo: HashMap<usize, Value>,
      jobs: Vec<CloneJob>,
      fills: Vec<PendingFill>,
  }

  enum CloneJob {
      NativeArray {
          source: SharedArray,
          target: SharedArray,
      },
      NativeObject {
          source: SharedObject,
          target: SharedObject,
      },
      HostArray {
          source: Rc<HostLease>,
          target: SharedArray,
      },
      HostObject {
          source: Rc<HostLease>,
          target: SharedObject,
      },
  }

  enum PendingFill {
      Array {
          target: SharedArray,
          values: Vec<Value>,
          holes: HashSet<usize>,
      },
      Object {
          target: SharedObject,
          fields: IndexMap<Vec<u8>, Value>,
      },
  }
  ```

- [ ] **Step 1: 自己循環を固定する失敗テストを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_preserves_a_self_cycle()
  ```

  Test construction and assertions:

  ```rust
  let source = Value::shared_object(IndexMap::new());
  let Value::Object(source_fields) = &source else { unreachable!() };
  shared_object_ref_mut(source_fields).insert(b"self".to_vec(), source.clone());

  let cloned = clone_value(&source, CloneMode::Structured).unwrap();

  assert!(!strict_identity(&source, &cloned));
  let cloned_self = cloned
      .as_object()
      .unwrap()
      .get(b"self".as_slice())
      .unwrap();
  assert!(strict_identity(&cloned, cloned_self));
  ```

  `strict_identity`はtest-only helperとし、Array/Objectは`Rc::ptr_eq`、Hostはcanonical handle、scalarは値比較を使う。

- [ ] **Step 2: 共有child identityとsource independenceの失敗テストを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_preserves_shared_children_but_not_source_identity()
  ```

  Assertions:

  ```rust
  clone.left === clone.right
  clone.left !== source.left
  clone !== source
  clone.left.x を変更しても source.left.x は 1 のまま
  clone.right.x は変更後の値を観測する
  ```

- [ ] **Step 3: Object↔Array相互循環とholeの失敗テストを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_preserves_mixed_cycles_and_array_holes()
  ```

  Graph:

  ```text
  root.list -> array
  array[0] -> root
  array[1] -> hole
  array[2] -> explicit undefined sentinel
  ```

  Assertions:

  ```text
  clone.list[0] is clone root
  index 1 is absent
  index 2 is present and reads undefined
  cloned list is not source list
  ```

- [ ] **Step 4: 深い非循環graphの失敗テストを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_handles_a_4096_node_chain_without_recursion()
  ```

  `Value::shared_object`をloopで4,096段構築し、clone後もloopで末端まで辿る。再帰helperと`JSON.stringify`はtest内で使用しない。

- [ ] **Step 5: REDを確認する**

  Run:

  ```bash
  cargo test -p thaw-std structured_clone_ --locked -- --nocapture --test-threads=1
  ```

  Expected:

  - self-cycle testはstack overflowまたはprocess failureを再現する。
  - shared-child testは`clone.left !== clone.right`で失敗する。
  - 4,096段testは旧再帰実装ではstack依存となる。
  - failure理由がtest fixtureの誤りではなく、旧`deep_clone_mode`にあることを確認する。

- [ ] **Step 6: `GraphCloner::intern`を実装する**

  Signature:

  ```rust
  impl GraphCloner {
      fn intern(&mut self, source: &Value) -> Result<Value, ()>;
  }
  ```

  Rules:

  - scalarは値copyを返す。
  - Array/Objectは`object_identity_key`でmemo lookupする。
  - miss時は**空shellを先に生成してmemoへ登録**し、その後`CloneJob`をqueueする。
  - native Object shellは、現実装と同様にsourceの`napi_lease`を`Rc::clone`する。
  - wrapper brandとgraph-plain markerはclone shellへ複製する。
  - `native_links`、prototype、object-stateは複製しない。
  - Task 1ではHost branchを既存挙動へ委譲し、Task 2でmemo-awareに置換する。

- [ ] **Step 7: discovery phaseを実装する**

  Signature:

  ```rust
  impl GraphCloner {
      fn discover(mut self, source: &Value) -> Result<(Value, Vec<PendingFill>), ()>;
      fn process_job(&mut self, job: CloneJob) -> Result<(), ()>;
  }
  ```

  Native Array/Objectについて:

  - source contentをshallow snapshotする。
  - 各childへ`intern`を適用する。
  - target shellはまだ変更せず、`PendingFill`へ完成予定の辺を保存する。
  - array hole setもこの時点でsnapshotする。後のHost reentryによるsource mutationの影響をcommit時に受けないようにする。

- [ ] **Step 8: commit phaseを実装する**

  Signature:

  ```rust
  fn commit_fills(fills: Vec<PendingFill>);
  ```

  Rules:

  - discoveryが全件成功した後だけ実行する。
  - Arrayはtargetの空`Vec`を完成済み`values`へ置換し、snapshot済みhole setを登録する。
  - Objectはtargetの空`IndexMap`を完成済み`fields`へ置換する。
  - commit中はHost query、allocation、user callbackを実行しない。
  - commit後にのみcycleが形成されるため、discovery失敗時は通常のdropだけで全shellを回収できる。

- [ ] **Step 9: `json.rs`の旧再帰実装を委譲へ置き換える**

  Required shape:

  ```rust
  fn deep_clone(&self) -> Value {
      structured_clone::clone_value(self, CloneMode::Snapshot)
          .expect("snapshot clone cannot invoke a failing host operation")
  }

  fn deep_clone_mode(&self, live: bool) -> Value {
      let mode = if live {
          CloneMode::Structured
      } else {
          CloneMode::Snapshot
      };
      structured_clone::clone_value(self, mode)
          .unwrap_or(Value::Null)
  }
  ```

  FFI exportはTask 3でerror returnを最終調整するため、Task 1では既存ABIを保つ。

- [ ] **Step 10: GREENを確認する**

  Run:

  ```bash
  cargo test -p thaw-std structured_clone_ --locked -- --nocapture --test-threads=1
  ```

  Expected: 新規4 testsがPASS。

- [ ] **Step 11: 既存clone／graph／destroy testsを確認する**

  Run:

  ```bash
  cargo test -p thaw-std \
    graph_transport_preserves_cycle_alias_and_date_node_identity \
    --locked -- --nocapture --test-threads=1

  cargo test -p thaw-std \
    graph_tokens_distinguish_napi_and_user_reserved_shapes \
    --locked -- --nocapture --test-threads=1
  ```

  Expected: PASS。N-API lease retain/release countも既存期待値のまま。

- [ ] **Step 12: commitする**

  ```bash
  git add crates/thaw-std/src/json.rs \
          crates/thaw-std/src/json/structured_clone.rs
  git commit -m "fix(std): clone dynamic values as identity graphs"
  ```

---

### Task 2: Plain Host graphのcycle・alias・failure atomicityを実装する

**Files:**
- Modify: `crates/thaw-std/src/json/structured_clone.rs`
- Modify: `crates/thaw-std/src/json.rs`
- Test: `crates/thaw-std/src/json/structured_clone.rs`
- Test: `crates/thaw-std/src/json.rs`の既存Host fixture群を必要最小限再利用

**Interfaces:**
- Consumes:
  - Task 1の`GraphCloner`
  - `Value::Host(Rc<HostLease>)`
  - `object_identity_key`のtagged Host handle identity
  - `host_query`
  - `host_enumerate_values`
  - `host_enumerable_entries`
  - `HOST_ERROR`
- Produces:
  ```rust
  enum HostCloneKind {
      Shared,
      Array,
      Object,
  }

  fn classify_host_for_clone(
      lease: &Rc<HostLease>,
      mode: CloneMode,
  ) -> Result<HostCloneKind, ()>;
  ```
- `Shared`はFunction、Date、Map、Setなど現状materialize対象外のHost。元leaseを共有する。
- `Array`／`Object`はempty native shellをmemoへ登録してから列挙する。

- [ ] **Step 1: 同一Host handleのaliasとself-cycleを固定する失敗テストを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_materializes_each_plain_host_identity_once()
  ```

  Fixture contract:

  - handle `700`はplain object。
  - enumerable properties:
    - `self` -> handle `700`
    - `left` -> handle `701`
    - `right` -> handle `701`
  - handle `701`はplain object `{ x: 1 }`。
  - query/enumerate callbackはhandleごとの呼出し回数を記録する。

  Assertions:

  ```text
  clone.self === clone
  clone.left === clone.right
  clone.left is not Host handle 701
  handle 700 is enumerated once
  handle 701 is enumerated once
  source Host leases remain valid until source destruction
  ```

- [ ] **Step 2: Host array/object相互循環の失敗テストを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_materializes_mutually_recursive_host_array_and_object()
  ```

  Graph:

  ```text
  host object 710.list -> host array 711
  host array 711[0] -> host object 710
  ```

  Assertions:

  ```text
  clone.list[0] === clone
  clone.list is a native Array
  clone is a native Object
  ```

- [ ] **Step 3: Host enumeration失敗のatomicity testを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_host_failure_discards_uncommitted_shells_and_leases_once()
  ```

  Fixture:

  - root handle `720`の最初のpropertyはchild handle `721`。
  - 二つ目のproperty取得／enumerationで`"clone getter exploded"`を返す。
  - release callbackはhandle別countを記録し、任意で別`thaw_json_destroy`へreenterする。

  Assertions:

  ```text
  clone_value returns Err(())
  thaw_json_take_host_error() == "clone getter exploded"
  no target native container remains externally reachable
  each temporary Host retain/release is balanced exactly once
  cleanup-only error does not overwrite "clone getter exploded"
  a second ordinary clone succeeds, proving no stale HOST_ERROR
  ```

- [ ] **Step 4: REDを確認する**

  Run:

  ```bash
  cargo test -p thaw-std \
    structured_clone_materializes_ \
    --locked -- --nocapture --test-threads=1

  cargo test -p thaw-std \
    structured_clone_host_failure_ \
    --locked -- --nocapture --test-threads=1
  ```

  Expected: 旧`host_deep_clone`はcycleで再帰し、同じhandleを複数回materializeし、failure atomicityを満たさない。

- [ ] **Step 5: Host classificationを実装する**

  Query orderは現実装を維持する。

  ```text
  typeof query
  null query
  function/non-plain query
  array query
  ```

  各query後に`HOST_ERROR`を確認し、errorがあれば`Err(())`を返す。

- [ ] **Step 6: Host shellをmemoへ先行登録する**

  `intern`のHost branch:

  - `Snapshot`なら`Value::Host(Rc::clone(lease))`。
  - `Structured + Shared`もlease共有。
  - `Structured + Array/Object`はtagged handle identityでmemo lookup。
  - miss時はempty native Array/Object shellを作成し、memo登録後に`HostArray`／`HostObject` jobをqueueする。
  - 列挙はmemo登録後に行うため、self-cycleが同じtarget shellへ戻る。

- [ ] **Step 7: Host jobをstaged fillへ変換する**

  - `host_enumerate_values`／`host_enumerable_entries`を一度だけ実行。
  - 戻り値の各childを`intern`。
  - target shellはdiscovery中に変更しない。
  - Host error発生時は即`Err(())`。
  - 既存`host_deep_clone`は削除し、すべてGraphClonerへ一本化する。

- [ ] **Step 8: GREENを確認する**

  Run:

  ```bash
  cargo test -p thaw-std structured_clone_ \
    --locked -- --nocapture --test-threads=1
  ```

  Expected: Task 1とTask 2の全structured clone unit testsがPASS。

- [ ] **Step 9: Host／N-API ownership回帰を確認する**

  Run:

  ```bash
  cargo test -p thaw-std host_collection_snapshot_ \
    --locked -- --nocapture --test-threads=1

  cargo test -p thaw-std callback_origin_ \
    --locked -- --nocapture --test-threads=1

  cargo test -p thaw-std graph_tokens_distinguish_napi_and_user_reserved_shapes \
    --locked -- --nocapture --test-threads=1
  ```

  Expected: PASS、retain/release count不変。

- [ ] **Step 10: commitする**

  ```bash
  git add crates/thaw-std/src/json.rs \
          crates/thaw-std/src/json/structured_clone.rs
  git commit -m "fix(std): preserve host graph identity during structured clone"
  ```

---

### Task 3: Runtime failureをLLVM例外経路へ即時伝播する

**Files:**
- Modify: `crates/thaw-std/src/json.rs`
- Modify: `crates/thaw-llvm/src/hir_codegen/invocations/json_calls.rs`
- Test: `crates/thaw-llvm/src/hir_codegen/tests/values/dynamic_and_json.rs`

**Interfaces:**
- Consumes:
  - `structured_clone::clone_value`
  - `compile_check_json_host_error`
- Produces:
  - `thaw_json_structured_clone`は成功時にowned `*mut Value`、失敗時にnullを返し、原因は`HOST_ERROR`に残す。
  - LLVM intrinsic pathはclone call直後にHost errorをconsumeする。

- [ ] **Step 1: LLVM IR orderingの失敗テストを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_checks_host_error_before_using_the_result()
  ```

  Source:

  ```ts
  function main(value: any): void {
      structuredClone(value);
  }
  ```

  Compile to LLVM IR and assert:

  ```text
  call @thaw_json_structured_clone
  から4 instruction以内に
  call @thaw_json_take_host_error
  がある。

  failure blockに
  call @thaw_json_destroy
  がある。

  failure blockはpending exceptionへbranchする。
  ```

- [ ] **Step 2: discarded success resultのownership testを書く**

  同じIR testで、success pathに既存のfresh-owned result cleanupが残ることをassertする。

  ```text
  destroy_untransferred_json_result block exists
  call @thaw_json_destroy exists
  named bindingへ代入した場合はcleanup candidateがrevokeされる
  ```

- [ ] **Step 3: REDを確認する**

  Run:

  ```bash
  cargo test -p thaw-llvm \
    structured_clone_checks_host_error_before_using_the_result \
    --locked -- --nocapture --test-threads=1
  ```

  Expected: 現状は`thaw_json_take_host_error`がclone直後に存在せずFAIL。

- [ ] **Step 4: FFI exportのfailure returnを明確化する**

  Required behavior:

  ```rust
  pub unsafe extern "C" fn thaw_json_structured_clone(
      value: *const Value,
  ) -> *mut Value {
      let Some(value) = value.as_ref() else {
          return leak(Value::Null);
      };
      match structured_clone::clone_value(value, CloneMode::Structured) {
          Ok(clone) => leak(clone),
          Err(()) => std::ptr::null_mut(),
      }
  }
  ```

  `thaw_json_clone`は`CloneMode::Snapshot`を使用し、Host callbackを呼ばないためinfallibleとして維持する。

- [ ] **Step 5: LLVM intrinsicへHost error checkを追加する**

  `__thaw_json_clone` branchを次の順序にする。

  ```rust
  let result = self.compile_single_arg_call(
      "thaw_json_structured_clone",
      args,
      "structuredClone",
  )?;
  self.compile_check_json_host_error(
      result,
      Some("thaw_json_destroy"),
  )
  ```

  error check前にresultへproperty access、ownership transfer、次のHost callを行わない。

- [ ] **Step 6: GREENを確認する**

  Run:

  ```bash
  cargo test -p thaw-llvm \
    structured_clone_checks_host_error_before_using_the_result \
    --locked -- --nocapture --test-threads=1
  ```

  Expected: PASS。

- [ ] **Step 7: runtime clone unit testsを再確認する**

  Run:

  ```bash
  cargo test -p thaw-std structured_clone_ \
    --locked -- --nocapture --test-threads=1
  ```

  Expected: PASS。

- [ ] **Step 8: commitする**

  ```bash
  git add crates/thaw-std/src/json.rs \
          crates/thaw-llvm/src/hir_codegen/invocations/json_calls.rs \
          crates/thaw-llvm/src/hir_codegen/tests/values/dynamic_and_json.rs
  git commit -m "fix(llvm): propagate structured clone host failures"
  ```

---

### Task 4: End-to-endでNode互換のcycle・alias・errorを固定する

**Files:**
- Modify: `crates/thaw-llvm/src/hir_codegen/tests/values/dynamic_and_json.rs`
- Modify: `tests/builtins-compat.json`

**Interfaces:**
- Consumes: Task 1〜3のruntime／LLVM経路
- Produces: user-visible behaviorを固定するcompiler testsとNode differential case

- [ ] **Step 1: native dynamic graphのend-to-end失敗テストを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_preserves_cycles_and_shared_identity()
  ```

  Source:

  ```ts
  function main(): void {
      const shared: any = { x: 1 };
      const source: any = { left: shared, right: shared };
      source.self = source;

      const clone: any = structuredClone(source);
      clone.left.x = 9;

      console.log(
          clone !== source,
          clone.self === clone,
          clone.left === clone.right,
          clone.left !== shared,
          source.left.x,
          clone.right.x,
      );
  }
  ```

  Expected output:

  ```text
  true true true true 1 9
  ```

- [ ] **Step 2: Object↔Array cycleのend-to-end testを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_preserves_mixed_object_array_cycles()
  ```

  Assertions:

  ```text
  clone.list[0] === clone
  clone.list !== source.list
  sparse hole remains absent
  explicit undefined slot remains present
  ```

- [ ] **Step 3: live plain Host graphのend-to-end testを書く**

  QuickJSでgraphを作る。

  ```ts
  loadScript(`
      const child = { x: 1 };
      const root = { left: child, right: child };
      root.self = root;
      globalThis.cloneSource = root;
  `);

  const source: any = getDynamicValue("cloneSource");
  const clone: any = structuredClone(source);
  ```

  Expected:

  ```text
  clone.self === clone
  clone.left === clone.right
  clone.left !== source.left
  ```

- [ ] **Step 4: getter failureと次回成功を固定するtestを書く**

  Test name:

  ```rust
  #[test]
  fn structured_clone_propagates_host_getter_failure_without_stale_state()
  ```

  Source behavior:

  ```ts
  loadScript(`
      globalThis.failingCloneSource = Object.defineProperty({}, "boom", {
          enumerable: true,
          get() { throw new TypeError("clone getter exploded"); }
      });
      globalThis.goodCloneSource = { ok: 1 };
  `);

  try {
      structuredClone(getDynamicValue("failingCloneSource"));
  } catch (error: any) {
      console.log(error.name, error.message);
  }

  const good: any = structuredClone(getDynamicValue("goodCloneSource"));
  console.log(good.ok);
  ```

  Expected output:

  ```text
  TypeError clone getter exploded
  1
  ```

- [ ] **Step 5: REDを確認する**

  Task 4 testsをTask 1実装前に追加する場合は旧実装でFAILを確認する。Task 1〜3後に追加する場合は、audit reproducer manifestを旧SHAで一度実行した記録をRED証跡としてplan logへ残す。

  Run:

  ```bash
  cargo test -p thaw-llvm structured_clone_preserves_ \
    --locked -- --nocapture --test-threads=1

  cargo test -p thaw-llvm structured_clone_propagates_ \
    --locked -- --nocapture --test-threads=1
  ```

- [ ] **Step 6: compatibility manifestへcaseを追加する**

  Add:

  ```json
  {
    "group": "structuredClone",
    "name": "structuredClone.cycle-and-shared-identity",
    "expect": "matched",
    "source": "function main(): void { /* Task 4 Step 1と同じobservable assertions */ }"
  }
  ```

  OutputはNodeとThaw双方で同じ文字列になるよう、cycle自体を`JSON.stringify`しない。

- [ ] **Step 7: targeted compatibilityを実行する**

  まず新caseだけを一時manifestへ抽出して実行する。

  ```bash
  jq '{
    cases: [
      .cases[]
      | select(.name == "structuredClone.cycle-and-shared-identity")
    ]
  }' tests/builtins-compat.json \
    > /tmp/structured-clone-compat.json

  cargo run -p thaw-cli --locked -- \
    node-compat /tmp/structured-clone-compat.json
  ```

  Expected:

  ```text
  total: 1
  bugs: 0
  matched: 1
  ```

- [ ] **Step 8: 全structuredClone compiler testsを実行する**

  ```bash
  cargo test -p thaw-llvm structured_clone \
    --locked -- --nocapture --test-threads=1
  ```

  Expected: 全件PASS。

- [ ] **Step 9: commitする**

  ```bash
  git add crates/thaw-llvm/src/hir_codegen/tests/values/dynamic_and_json.rs \
          tests/builtins-compat.json
  git commit -m "test: cover structured clone graph identity end to end"
  ```

---

### Task 5: Regression、静的検査、failure cleanup監査

**Files:**
- Modify only if verification reveals a defect:
  - `crates/thaw-std/src/json/structured_clone.rs`
  - `crates/thaw-std/src/json.rs`
  - `crates/thaw-llvm/src/hir_codegen/invocations/json_calls.rs`
- Test all files changed above

**Interfaces:**
- Consumes: completed implementation
- Produces: merge可能なbranchと検証記録

- [ ] **Step 1: formatする**

  ```bash
  cargo fmt --all
  cargo fmt --all -- --check
  ```

  Expected: PASS。

- [ ] **Step 2: focused Clippyを実行する**

  ```bash
  cargo clippy -p thaw-std -p thaw-llvm --all-targets --locked -- -D warnings
  ```

  Expected: PASS、warning 0。

- [ ] **Step 3: focused testsを実行する**

  ```bash
  cargo test -p thaw-std structured_clone_ \
    --locked -- --nocapture --test-threads=1

  cargo test -p thaw-llvm structured_clone \
    --locked -- --nocapture --test-threads=1
  ```

  Expected: PASS。

- [ ] **Step 4: full `thaw-std`と`thaw-llvm` testsを実行する**

  ```bash
  cargo test -p thaw-std --locked -- --test-threads=1
  cargo test -p thaw-llvm --locked -- --test-threads=1
  ```

  Expected: PASS。

- [ ] **Step 5: compatibility suitesを実行する**

  ```bash
  cargo run -p thaw-cli --locked -- \
    node-compat tests/typescript-runtime-compat.json

  cargo run -p thaw-cli --locked -- \
    node-compat tests/node-compat.json

  cargo run -p thaw-cli --locked -- \
    node-compat tests/builtins-compat.json
  ```

  Expected:

  - 新structuredClone caseは`matched`。
  - 既存expected-matched caseに新しいregressionがない。
  - 既知の別問題が残る場合、今回のbranchによる増加がないことをcase名付きで記録する。

- [ ] **Step 6: full workspace verificationを実行する**

  ```bash
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo test --workspace --locked -- --test-threads=1
  ```

  Expected: PASS。既存環境要因によるfailureがある場合は、command、test名、今回の変更との因果を明示する。

- [ ] **Step 7: clone failure cleanupをレビューする**

  Review checklist:

  ```text
  [ ] shellはmemo登録後、全discovery成功まで空のまま
  [ ] discovery error時にcommitされない
  [ ] Host/N-API leaseはexactly once release
  [ ] failure resultはLLVM側でdestroy
  [ ] HOST_ERRORは次のoperationへ残らない
  [ ] success cycleはthaw_json_destroyで回収可能
  [ ] native_links/prototype/object stateを誤って共有していない
  ```

- [ ] **Step 8: 最終差分を確認する**

  ```bash
  git status --short
  git diff --check
  git log --oneline --decorate -5
  git diff main...HEAD --stat
  ```

  Expected:

  - plan対象外のfile変更なし。
  - whitespace errorなし。
  - commitがTask単位で分かれている。

- [ ] **Step 9: verification修正があればcommitする**

  ```bash
  git add <verificationで修正した対象file>
  git commit -m "fix: harden structured clone graph cleanup"
  ```

  修正不要なら空commitは作らない。

---

### Task 6: Branch handoffとPR準備

**Files:**
- Create in repository only when implementation begins:
  - `docs/superpowers/plans/2026-10-19-structured-clone-graph-identity.md`
- No production changes

**Interfaces:**
- Consumes: green branch
- Produces: review可能なhandoff

- [ ] **Step 1: plan documentをrepositoryへ保存する**

  このplanと同一内容を次へ置く。

  ```text
  docs/superpowers/plans/2026-10-19-structured-clone-graph-identity.md
  ```

- [ ] **Step 2: planと検証結果をcommitする**

  ```bash
  git add docs/superpowers/plans/2026-10-19-structured-clone-graph-identity.md
  git commit -m "docs: record structured clone graph fix plan"
  ```

- [ ] **Step 3: branch summaryを作る**

  Summary must include:

  ```text
  base SHA
  branch HEAD
  root cause
  algorithm
  changed files
  targeted test results
  full test results
  compatibility counts
  known non-goals
  ```

- [ ] **Step 4: push前にremote mainとの差を再確認する**

  GitHub connectorまたはHTTPS fetchで`main` HEADを確認する。基点から進んでいた場合は、clean worktree内でrebaseし、Task 5のverificationを再実行する。

- [ ] **Step 5: push／PR用の最終状態を確認する**

  ```bash
  git status --short
  git branch --show-current
  git rev-parse HEAD
  ```

  Expected:

  ```text
  clean worktree
  branch = fix/structured-clone-graph-identity
  HEAD recorded in handoff
  ```

---

## Acceptance Criteria

実装は次をすべて満たした時だけ完了とする。

1. `structuredClone`でself-cycle、mutual cycleがstack overflowしない。
2. clone内の共有参照が維持される。
3. cloneとsourceのcontainer identityは分離される。
4. clone側のmutationがsourceへ到達しない。
5. sparse arrayのholeと明示`undefined`が区別される。
6. 4,096段の非循環graphを再帰stackなしでcloneできる。
7. plain Host cycle／aliasがcanonical handleごとに一度だけmaterializeされる。
8. Host getter／enumeration失敗が元のError name/messageでcatch可能。
9. failure後に次のcloneが成功し、stale `HOST_ERROR`がない。
10. failure時のtemporary Host/N-API leaseがexactly onceでreleaseされる。
11. success cycleを`thaw_json_destroy`でき、既存cycle cleanupを壊さない。
12. 既存structuredClone tests、`thaw-std`、`thaw-llvm`、compatibility suitesに新規regressionがない。
13. 公開TypeScript APIとC ABIに変更がない。

## Explicit Non-Goals

- transfer listの実装
- Functionのclone対応
- Date、RegExp、Error、Map、Set等のHost objectを完全なstructured clone対象へ拡張
- prototype／property descriptor／frozen stateのclone
- typed native Mapのobject key clone対応
- `tests/builtins-compat.json`をCI workflowへ追加する作業
- 他の監査項目R1〜R5、R7〜R11の同時修正
