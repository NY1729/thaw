// Anonymous async and generator functions print their kind like Node (`[AsyncFunction (anonymous)]`);
// named and sync anonymous functions keep their labels. Expected output recorded from node 22.

#[test]
fn anonymous_async_and_generator_functions_print_their_kind() {
    let source = r##"function main(): void {
  console.log([async () => 1, async function () { await 0; }, function* () {}, async function* () {}]);
}
"##;
    assert_eq!(
        compile_and_run(source, "anonymous_async_and_generator_functions_print_their_kind"),
        "[\n  [AsyncFunction (anonymous)],\n  [AsyncFunction (anonymous)],\n  [GeneratorFunction (anonymous)],\n  [AsyncGeneratorFunction (anonymous)]\n]\n"
    );
}

#[test]
fn named_and_sync_anonymous_function_labels_are_unchanged() {
    let source = r##"function main(): void {
  const named = async () => 5;
  const gen = function* () {};
  console.log(named);
  console.log(gen);
  console.log(() => 6);
  console.log(async () => 1);
  console.log(function* () {});
}
"##;
    assert_eq!(
        compile_and_run(source, "named_and_sync_anonymous_function_labels_are_unchanged"),
        "[AsyncFunction: named]\n[GeneratorFunction: gen]\n[Function (anonymous)]\n[AsyncFunction (anonymous)]\n[GeneratorFunction (anonymous)]\n"
    );
}

#[test]
fn anonymous_async_function_in_array_and_class_field_labels() {
    let source = r##"class K {
  f: () => Promise<number> = async () => 4;
}
function main(): void {
  console.log(new K());
  const items: Array<() => Promise<number>> = [async () => 7];
  console.log(items);
}
"##;
    assert_eq!(
        compile_and_run(source, "anonymous_async_function_in_array_and_class_field_labels"),
        "K { f: [AsyncFunction: f] }\n[ [AsyncFunction (anonymous)] ]\n"
    );
}
