# 例外/Errorモデル設計ドキュメント

- ステータス: 単一タグ付き文字列としての例外チャンネルはV1として実装済み
  （`new Error(message)`/`new TypeError(...)`/等がクラス名を
  `\u{1}<Name>\u{1}<message>`という形でメッセージ文字列自身にタグ付けし、
  `.message`/`.name`/`instanceof Error`系がそのタグを読む。`Promise.resolve`/
  `Promise.reject`もこの文字列チャンネルの上に実装済み。N-API境界
  （`napi_create_error`系→`napi_throw`）を越えるタグ保持も実装済み。非文字列の
  `throw`は同じ文字列変換（`String(value)`が使うものと同一）を通るため、
  クラッシュではなくコンパイルエラーか安全な文字列化になる）。
  **V1(3.3節・6節)も実装済み**: `class MyError extends Error { code: number; ... }`
  は既存のクラス/オブジェクト機構をそのまま再利用する形でコンパイルできる
  ようになった。フィールドはthrowされるまで(オブジェクトのままの間)は
  普通に読み書きでき、throw後は`.message`/`.name`/`instanceof`(多段継承の
  祖先も含めて)が正しく動く。`code`のような独自フィールドをcatch後に
  読む(`if (e instanceof MyError) { e.code }`)ことはまだできない --
  それが3.1〜3.2節で提案した`_object`並行チャンネル(V2、未実装)の役割。
- 前提: [thaw-hir/src/lower/expressions/lowering.rs](../../crates/thaw-hir/src/lower/expressions/lowering.rs)
  の`new Error`系特殊扱いと`instanceof`特殊扱い、
  [thaw-hir/src/lower/module/classes.rs](../../crates/thaw-hir/src/lower/module/classes.rs)
  のネイティブクラスレイアウト解決、
  [thaw-runtime](../../crates/thaw-runtime/src/runtime/native_values/errors.rs)の
  `split_error_tag`/`thaw_error_name`/`thaw_error_message`/`thaw_error_is_instance`
  を前提にする。

## 1. 目的とスコープ

`class MyError extends Error { code: number; constructor(message, code) { super(message); this.code = code; } }`
のような、ユーザー定義クラスが`Error`(またはその子孫)を継承し、
独自フィールドを持てるようにする。継承元が`Error`系だと分かった上で:

- `throw new MyError("x", 42)` → `catch (e)`側で`e.message`/`e.name`/
  `e instanceof MyError`/`e instanceof Error`が全て動く(既存の文字列タグと
  同じ挙動)
- **加えて** `if (e instanceof MyError) { console.log(e.code); }`のように、
  ナローイング後に**ユーザー定義フィールドへ実際にアクセスできる**

後者が今回の本質的な追加要求であり、既存の「メッセージ+名前だけの文字列タグ」
では原理的に表現できない。

## 2. なぜ難しいか: 2つの値表現の衝突

Thawの値表現は([async-await.md](async-await.md)3.2節の要約通り)
`f64` / `bool` / 文字列・配列・オブジェクトはポインタ、の4種類に単純化されて
いる。**文字列とオブジェクトはどちらも生ポインタとして表現される**が、
指す先のメモリレイアウトが全く違う:

- **文字列**(`HirType::Str`): NUL終端のUTF-8バイト列。例外チャンネル
  (`__thaw_pending_exception`グローバル、[hir_codegen.rs:86](../../crates/thaw-llvm/src/hir_codegen.rs))
  は今このi8*一本だけを持つ。
- **オブジェクト**(`HirType::Object(fields)`): 固定レイアウトの構造体。先頭
  フィールドは`__thaw_class_identity_<Name>$<Name2>$...`という`bool`マーカー
  で、継承チェーンの識別に使われる
  ([module/classes.rs:50-56](../../crates/thaw-hir/src/lower/module/classes.rs))。
  `instanceof`は**コンパイル時**に静的型とこのマーカー名を照合するだけで
  (`class_type_has_identity`)、実行時チェックを一切しない。

`catch (e)`は現状常に`e`を`HirType::Str`として束縛する
([statements/lowering.rs:857](../../crates/thaw-hir/src/lower/statements/lowering.rs))。
これは「例外は常に何かの文字列である」という前提に立っており、正しい
(実際、プレーンな`throw "x"`やビルトインError系はこれで十分)。しかし
`MyError`のような**フィールドを持つオブジェクト**を投げた場合、`e`の実体は
文字列ではなくオブジェクトへのポインタになるべきで、そのままでは
既存の「eは常にHirType::Str」という前提が崩れる。

さらに、`instanceof`が今は**コンパイル時解決**である点も問題になる。
`catch (e)`の`e`は「関数のどこかでthrowされた可能性のある、型が静的には
決まらない値」なので、`e instanceof MyError`は本質的に**実行時チェック**
にならざるを得ない(実際、Error系はすでにこの理由で
`thaw_error_is_instance`という実行時関数に迂回している -- ちょうど今回
拡張したい先例)。

## 3. 設計方針: 既存チャンネルを壊さず横に足す

大きな作り直し(例外を完全にオブジェクトへ置き換える)は、この1本の文字列
チャンネルに依存している既存の全経路
(console.log表示・N-API境界・Lambdaエラー報告・`Promise.reject`)を
同時に書き換えることになり、リスクが高い。代わりに、**既存の文字列
チャンネルはそのまま残し、フィールドを持つ例外のときだけ並行するオブジェクト
チャンネルを追加で使う**、という設計を提案する。

### 3.1 2本立てのチャンネル

```
__thaw_pending_exception        : i8*   (既存、変更なし)
__thaw_pending_exception_object : i8*   (新規、nullable)
```

- `throw`されたのが**プレーンな文字列**、または**フィールドを持たない
  `Error`系**(ビルトイン、またはフィールドなしでユーザー定義された
  `class MyError extends Error {}`)のときは、今まで通り
  `__thaw_pending_exception`だけをセットする。`_object`は`null`のまま。
- `throw`されたのが**フィールドを持つ、`Error`系を継承したユーザークラスの
  インスタンス**のときは、
  1. `__thaw_pending_exception_object`にそのオブジェクトの生ポインタを
     セットする
  2. `__thaw_pending_exception`にも**同時に**、そのクラス名と`message`
     フィールドから組み立てたタグ付き文字列(`\u{1}<Name>\u{1}<message>`)
     をセットする(オブジェクトから文字列表現を都度導出するだけで、
     既存の`split_error_tag`系ヘルパーは一切変更不要)

この二重化により、**既存の全ての読み手(console.log・N-API・Lambdaエラー
報告・`.message`/`.name`/`instanceof Error`系)は`_object`の存在を一切
知らなくてもよい** -- 常に`__thaw_pending_exception`(文字列)だけを見れば
今まで通り正しく動く。`_object`は「ユーザー定義フィールドを読みたい」という
**新しい**要求のためだけに追加された、完全にオプトインの経路になる。

### 3.2 `catch (e)`と型ナローイング

`catch (e)`の束縛型は今まで通り`HirType::Str`のままにする(変更しない)。
`e instanceof MyError`は既存の`thaw_error_is_instance`と同じ実行時タグ
比較で判定するが、**加えて**このナローイングが成立した分岐内でだけ、
`e`を`MyError`型として再解釈した**別の隠し変数**
(`__thaw_narrowed_exception_object`のような)を導入し、
`__thaw_pending_exception_object`退避値(catch時点でarena上にコピー
しておいたもの)を`MyError`のポインタ型にキャストして束縛する。

```ts
try {
    throw new MyError("x", 42);
} catch (e) {
    if (e instanceof MyError) {
        console.log(e.code);   // ← ここだけ __thaw_pending_exception_object 経由
    }
    console.log(e.message);    // ← 従来通り __thaw_pending_exception 経由
}
```

これは、既存の`union_narrowings`/`class_type_has_identity`ナローイング
機構([expressions/lowering.rs](../../crates/thaw-hir/src/lower/expressions/lowering.rs)の
`instanceof`分岐、および`self.narrowings`まわり)と同じ形の「分岐内だけ型が
変わる」パターンの応用で、新しい汎用機構を作らずに既存の型ファクト追跡へ
乗せられる可能性が高い。

### 3.3 クラス側の変更点

`module/classes.rs`の`resolve_layout`は今、基底クラス名が
`declarations`(ユーザー宣言のクラス一覧)に無いと即座に
`extends unknown native class`エラーを返す。ここに
「基底クラス名が`Error`/`TypeError`/`RangeError`/`SyntaxError`/
`ReferenceError`/`EvalError`/`URIError`のいずれかなら、`declarations`に
無くてもエラーにせず、代わりに**擬似的な基底レイアウト**
`HirType::Object([("__thaw_class_identity_<Name>", Bool), ("message", Str)])`
を合成する」という分岐を追加する。これにより:

- `MyError`のフィールドレイアウトは`[identity marker, message: Str, code: F64]`
  になり、既存の継承フィールド合成ロジック(56行目`inherited_fields.extend(...)`)
  がそのまま使える
- `super(message)`呼び出しは、通常のクラスなら基底コンストラクタ呼び出しに
  なるところを、「`message`フィールドへの代入 + `__thaw_pending_exception`用の
  タグ付き文字列は投げる瞬間に組み立てる」という特殊コード生成に差し替える

## 4. 影響範囲

- [thaw-hir/src/lower/module/classes.rs](../../crates/thaw-hir/src/lower/module/classes.rs):
  `Error`系を基底として許可するレイアウト合成の追加
- [thaw-hir/src/lower/expressions/lowering.rs](../../crates/thaw-hir/src/lower/expressions/lowering.rs):
  `new MyError(...)`(ユーザー定義Error系)のコンストラクタ呼び出し
  → オブジェクト構築 + タグ付き文字列の同時生成。`instanceof`のナローイング
  拡張
- [thaw-hir/src/lower/statements/lowering.rs](../../crates/thaw-hir/src/lower/statements/lowering.rs):
  `throw`時に「オブジェクトなら`_object`もセットする」分岐
- [thaw-llvm/src/hir_codegen.rs](../../crates/thaw-llvm/src/hir_codegen.rs):
  `__thaw_pending_exception_object`グローバルの追加宣言
- [thaw-llvm/src/hir_codegen/statements.rs](../../crates/thaw-llvm/src/hir_codegen/statements.rs):
  `compile_try`のcatch分岐で`_object`退避値も読む
- N-API境界・Lambdaエラー報告・`Promise.reject`は**変更不要**(3.1節参照)

## 5. 未解決の問題・リスク

- **クロスファンクション伝播**: `_object`もグローバル変数なので、文字列版と
  同じ「単一のpending state」方式で伝播自体は動くはずだが、非同期
  (`async`関数のフレーム分割)との相互作用は要検証。フレーム分割済みの
  関数がまたぐ場合に`_object`の生存期間(アリーナ)が正しいか要確認。
- **N-API側でユーザークラスをどう見せるか**: 現状はスコープ外にしている
  (3.1節)が、将来ネイティブアドオン側にユーザー定義Errorを渡したい場合は
  別途設計が要る。
- **`instanceof`ナローイング後のフィールドアクセスの型検査**: 「ナローイング
  が成立した分岐内でだけ`_object`経由アクセスを許可する」を型システムの
  どの層で強制するか(現在の`narrowings`まわりの機構で無理なく表現できるか)
  は、実装に着手して初めて確定する部分が大きい。
- **多段継承**(`class Sub extends MyError extends Error`): レイアウト合成
  ロジック自体は多段継承をすでに扱える設計だが、実際に確認していない。

## 6. 次のステップ

V1として提案するスコープ:

1. フィールドなしで`Error`系を継承するクラス(`class MyError extends Error {}`)
   が、既存のビルトインError系と全く同じ(文字列タグのみの)扱いでコンパイル
   できるようにする -- 3.3節のレイアウト合成のうち、独自フィールドが無い
   場合のみを先に通す
2. フィールドを持つ場合(3.1〜3.3節の`_object`チャンネル)を追加する

1だけでも「`instanceof`で自分のエラークラスを判別できる」という実用上の
価値の大部分を先に提供できるため、2より先に完了させる価値が高いと考える。
