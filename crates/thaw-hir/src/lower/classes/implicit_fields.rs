/// `this.name = ..` targets in a method body (not in nested `function`s or classes, which rebind `this`).
#[derive(Default)]
struct ThisAssignedNames(Vec<String>);

impl Visit for ThisAssignedNames {
    fn visit_assign_expr(&mut self, assign: &swc_ecma_ast::AssignExpr) {
        if let swc_ecma_ast::AssignTarget::Simple(swc_ecma_ast::SimpleAssignTarget::Member(member)) =
            &assign.left
        {
            if let (Expr::This(_), swc_ecma_ast::MemberProp::Ident(name)) =
                (member.obj.as_ref(), &member.prop)
            {
                self.0.push(name.sym.to_string());
            }
        }
        assign.visit_children_with(self);
    }
    fn visit_function(&mut self, _: &swc_ecma_ast::Function) {}
    fn visit_class(&mut self, _: &swc_ecma_ast::Class) {}
}

/// JavaScript classes declare no fields: every `this.x = ..` in a constructor or instance method
/// introduces an (untyped, so `any`) instance field.
struct DeclareAssignedFields;

impl VisitMut for DeclareAssignedFields {
    fn visit_mut_class(&mut self, class: &mut swc_ecma_ast::Class) {
        class.visit_mut_children_with(self);
        let mut assigned = ThisAssignedNames::default();
        let mut declared = HashSet::new();
        for member in &class.body {
            match member {
                ClassMember::Constructor(constructor) => {
                    if let Some(body) = &constructor.body {
                        body.visit_with(&mut assigned);
                    }
                }
                ClassMember::Method(method) => {
                    if let Some(name) = literal_property_name(&method.key) {
                        declared.insert(name.to_string());
                    }
                    if !method.is_static {
                        if let Some(body) = &method.function.body {
                            body.visit_with(&mut assigned);
                        }
                    }
                }
                ClassMember::ClassProp(property) => {
                    if let Some(name) = literal_property_name(&property.key) {
                        declared.insert(name.to_string());
                    }
                }
                ClassMember::PrivateProp(_) | ClassMember::PrivateMethod(_) => {}
                _ => {}
            }
        }
        for name in assigned.0 {
            if !declared.insert(name.clone()) {
                continue;
            }
            class.body.push(ClassMember::ClassProp(swc_ecma_ast::ClassProp {
                span: swc_common::DUMMY_SP,
                key: PropName::Ident(swc_ecma_ast::IdentName::new(name.as_str().into(), swc_common::DUMMY_SP)),
                value: None,
                type_ann: None,
                is_static: false,
                decorators: Vec::new(),
                accessibility: None,
                is_abstract: false,
                is_optional: false,
                is_override: false,
                readonly: false,
                declare: false,
                definite: false,
            }));
        }
    }
}

fn declare_assigned_class_fields(module: &Module) -> Module {
    let mut module = module.clone();
    module.visit_mut_with(&mut DeclareAssignedFields);
    module
}
