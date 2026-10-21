// Wave NB4: `yield*` into a user-defined iterator whose `throw` takes a string must
// deliver the thrown value's text, not an empty string. Expected output from node 22.

#[test]
fn nb4_contract_yield_delegate_passes_thrown_string_to_user_iterator() {
    let source = r##"function main(): void {
  const iterator = {
    next: function(): { value: number; done: boolean } { return { value: 1, done: false }; },
    throw: function(error: string): { value: number; done: boolean } {
      console.log("got:[" + error + "]");
      return { value: 2, done: false };
    },
  };
  const iterable = {
    [Symbol.iterator]: function(): { next: () => { value: number; done: boolean }; throw: (error: string) => { value: number; done: boolean } } {
      return iterator;
    },
  };
  const values = function* (): Generator<number> { yield* iterable; };
  const valuesIterator = values();
  console.log(valuesIterator.next().value);
  console.log(valuesIterator.throw("boom").value);
}"##;
    assert_eq!(
        compile_and_run(source, "nb4_contract_yield_delegate_passes_thrown_string_to_user_iterator"),
        "1\ngot:[boom]\n2\n"
    );
}
