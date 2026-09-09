#[derive(Clone)]
enum GeneratorTerm {
    Next(usize),
    Branch(HirExpr, usize, usize),
    Yield(HirStmt, usize),
    Done,
}

struct GeneratorBlock {
    statements: Vec<HirStmt>,
    term: GeneratorTerm,
}

struct GeneratorStateMachine<'a> {
    values: &'a str,
    blocks: Vec<GeneratorBlock>,
    locals: Vec<HirStmt>,
}

impl<'a> GeneratorStateMachine<'a> {
    fn new(values: &'a str) -> Self {
        Self {
            values,
            blocks: Vec::new(),
            locals: Vec::new(),
        }
    }

    fn block(&mut self, statements: Vec<HirStmt>, term: GeneratorTerm) -> usize {
        let id = self.blocks.len();
        self.blocks.push(GeneratorBlock { statements, term });
        id
    }

    fn sequence(
        &mut self,
        statements: &[HirStmt],
        continuation: usize,
        break_target: Option<usize>,
        continue_target: Option<usize>,
    ) -> Option<usize> {
        let mut next = continuation;
        for statement in statements.iter().rev() {
            next = self.statement(statement, next, break_target, continue_target)?;
        }
        Some(next)
    }

    fn statement(
        &mut self,
        statement: &HirStmt,
        continuation: usize,
        break_target: Option<usize>,
        continue_target: Option<usize>,
    ) -> Option<usize> {
        match statement {
            HirStmt::Let(name, ty, value) => {
                self.locals.push(HirStmt::Let(
                    name.clone(),
                    ty.clone(),
                    generator_placeholder(ty)?,
                ));
                Some(self.block(
                    vec![HirStmt::Expr(HirExpr::Assign(
                        name.clone(),
                        Box::new(value.clone()),
                    ))],
                    GeneratorTerm::Next(continuation),
                ))
            }
            HirStmt::If(condition, then_body, else_body) => {
                let then_entry =
                    self.sequence(then_body, continuation, break_target, continue_target)?;
                let else_entry =
                    self.sequence(else_body, continuation, break_target, continue_target)?;
                Some(self.block(
                    Vec::new(),
                    GeneratorTerm::Branch(condition.clone(), then_entry, else_entry),
                ))
            }
            HirStmt::While(condition, body) => {
                let condition_id = self.block(Vec::new(), GeneratorTerm::Done);
                let body_entry =
                    self.sequence(body, condition_id, Some(continuation), Some(condition_id))?;
                self.blocks[condition_id].term =
                    GeneratorTerm::Branch(condition.clone(), body_entry, continuation);
                Some(condition_id)
            }
            HirStmt::Break => Some(self.block(
                Vec::new(),
                GeneratorTerm::Next(break_target?),
            )),
            HirStmt::Continue => Some(self.block(
                Vec::new(),
                GeneratorTerm::Next(continue_target?),
            )),
            HirStmt::Return(_) => Some(self.block(Vec::new(), GeneratorTerm::Done)),
            HirStmt::Try(..) | HirStmt::BreakDepth(_) | HirStmt::ContinueDepth(_) => None,
            HirStmt::Expr(HirExpr::Assign(name, value))
                if name == self.values && matches!(value.as_ref(), HirExpr::ArrayConcat(..)) =>
            {
                // ponytail: delegated yields keep the eager path until the HIR can suspend
                // within a returned batch without advancing the producer.
                None
            }
            other if generator_emits_value(other, self.values) => Some(self.block(
                Vec::new(),
                GeneratorTerm::Yield(other.clone(), continuation),
            )),
            other => Some(self.block(
                vec![other.clone()],
                GeneratorTerm::Next(continuation),
            )),
        }
    }
}

fn generator_emits_value(statement: &HirStmt, values: &str) -> bool {
    match statement {
        HirStmt::Expr(HirExpr::Call(callee, arguments)) => {
            matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_array_push")
                && matches!(arguments.first(), Some(HirExpr::Var(name)) if name == values)
        }
        _ => false,
    }
}

fn generator_placeholder(ty: &HirType) -> Option<HirExpr> {
    Some(match ty {
        HirType::F64 => HirExpr::Lit(HirLit::F64(0.0)),
        HirType::I64 => HirExpr::Lit(HirLit::I64(0)),
        HirType::Bool => HirExpr::Lit(HirLit::Bool(false)),
        HirType::Str => HirExpr::Lit(HirLit::Str(String::new())),
        HirType::Array(_) => HirExpr::ArrayLit(Vec::new()),
        HirType::Optional(value) => HirExpr::OptionalNone(value.as_ref().clone()),
        HirType::Nullable(value) => HirExpr::NullableNone(value.as_ref().clone()),
        HirType::Nullish(value) => HirExpr::NullishUndefined(value.as_ref().clone()),
        HirType::Undefined => HirExpr::Lit(HirLit::Undefined),
        HirType::Null => HirExpr::Lit(HirLit::Null),
        _ => return None,
    })
}

fn lower_generator_state_machine(
    statements: &[HirStmt],
    values: &str,
    state: &str,
) -> Option<(usize, Vec<HirStmt>, Vec<HirStmt>)> {
    let mut machine = GeneratorStateMachine::new(values);
    let done = machine.block(Vec::new(), GeneratorTerm::Done);
    let entry = machine.sequence(statements, done, None, None)?;
    let mut dispatch = Vec::with_capacity(machine.blocks.len());
    for (id, block) in machine.blocks.into_iter().enumerate() {
        let mut selected = block.statements;
        match block.term {
            GeneratorTerm::Next(next) => {
                selected.push(generator_set_state(state, next));
                selected.push(HirStmt::Continue);
            }
            GeneratorTerm::Branch(condition, then_id, else_id) => {
                selected.push(HirStmt::If(
                    condition,
                    vec![generator_set_state(state, then_id)],
                    vec![generator_set_state(state, else_id)],
                ));
                selected.push(HirStmt::Continue);
            }
            GeneratorTerm::Yield(value, next) => {
                selected.push(value);
                selected.push(generator_set_state(state, next));
                selected.push(HirStmt::Return(Some(HirExpr::Var(values.into()))));
            }
            GeneratorTerm::Done => {
                selected.push(HirStmt::Expr(HirExpr::Assign(
                    state.into(),
                    Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                )));
                selected.push(HirStmt::Return(Some(HirExpr::Var(values.into()))));
            }
        }
        dispatch.push(HirStmt::If(
            HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(HirExpr::Var(state.into())),
                Box::new(HirExpr::Lit(HirLit::F64(id as f64))),
            ),
            selected,
            Vec::new(),
        ));
    }
    Some((
        entry,
        machine.locals,
        vec![HirStmt::While(
            HirExpr::Lit(HirLit::Bool(true)),
            dispatch,
        )],
    ))
}

fn generator_set_state(state: &str, next: usize) -> HirStmt {
    HirStmt::Expr(HirExpr::Assign(
        state.into(),
        Box::new(HirExpr::Lit(HirLit::F64(next as f64))),
    ))
}
