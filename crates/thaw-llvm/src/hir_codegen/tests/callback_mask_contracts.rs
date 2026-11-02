#[test]
fn jsvalue_param_mask_marks_bare_handle_positions_only() {
    use thaw_hir::HirType;
    // bare JsValue / Function / CallableFunction take the low bit of their position
    assert_eq!(jsvalue_param_mask(&[HirType::JsValue]), 1);
    assert_eq!(jsvalue_param_mask(&[HirType::Str, HirType::JsValue]), (1 << 32) | (1 << 1));
    assert_eq!(jsvalue_param_mask(&[HirType::F64, HirType::F64]), 0);
    // wrappers (Optional/Nullable/Nullish of JsValue) and Json stay out of the mask
    let wrapped = [
        HirType::Optional(Box::new(HirType::JsValue)),
        HirType::Nullable(Box::new(HirType::JsValue)),
        HirType::Nullish(Box::new(HirType::JsValue)),
        HirType::Json,
    ];
    for param in wrapped {
        assert_eq!(jsvalue_param_mask(std::slice::from_ref(&param)), 0, "{param:?}");
    }
    // `Str` positions set the high half, indexed from 32
    assert_eq!(jsvalue_param_mask(&[HirType::Str]), 1 << 32);
    // a method's receiver (NonArrowReceiver(JsValue)) at position 0 takes bit 0
    assert_eq!(
        jsvalue_param_mask(&[HirType::NonArrowReceiver(Box::new(HirType::JsValue))]),
        1
    );
}
