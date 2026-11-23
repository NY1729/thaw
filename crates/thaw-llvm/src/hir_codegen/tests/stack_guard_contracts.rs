// Process entry hands the compiled stack-limit slot to the host, so compiled code that later runs
// on a coroutine thread can have its limit swapped for that thread's stack (thaw-arena stack_guard).

#[test]
fn entry_registers_the_stack_limit_slot_after_storing_the_limit() {
    let (_, ir) = thaw_js_routed_ir("function main(): void { console.log(1); }");
    let limit = ir.find("call i64 @thaw_stack_guard_limit()").expect("limit read at entry");
    let register = ir
        .find("call void @thaw_stack_guard_register(ptr @__thaw_stack_limit)")
        .unwrap_or_else(|| panic!("slot not registered: {ir}"));
    assert!(limit < register, "register must follow the limit store: {ir}");
}
