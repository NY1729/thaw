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
    handler: Option<GeneratorHandler>,
    cancel_target: usize,
    forwards_control: bool,
}

#[derive(Clone)]
struct GeneratorHandler {
    binding: String,
    entry: usize,
}

struct GeneratorStateMachine<'a> {
    values: &'a str,
    finalizers: &'a HashMap<Symbol, Vec<HirStmt>>,
    blocks: Vec<GeneratorBlock>,
    locals: Vec<HirStmt>,
}

impl<'a> GeneratorStateMachine<'a> {
    fn new(values: &'a str, finalizers: &'a HashMap<Symbol, Vec<HirStmt>>) -> Self {
        Self {
            values,
            finalizers,
            blocks: Vec::new(),
            locals: Vec::new(),
        }
    }

    fn block(
        &mut self,
        statements: Vec<HirStmt>,
        term: GeneratorTerm,
        handler: Option<GeneratorHandler>,
        cancel_target: usize,
    ) -> usize {
        let id = self.blocks.len();
        self.blocks.push(GeneratorBlock {
            statements,
            term,
            handler,
            cancel_target,
            forwards_control: false,
        });
        id
    }

    fn sequence(
        &mut self,
        statements: &[HirStmt],
        continuation: usize,
        break_target: Option<usize>,
        continue_target: Option<usize>,
        handler: Option<GeneratorHandler>,
        cancel_target: usize,
    ) -> Option<usize> {
        let mut next = continuation;
        for statement in statements.iter().rev() {
            next = self.statement(
                statement,
                next,
                break_target,
                continue_target,
                handler.clone(),
                cancel_target,
            )?;
        }
        Some(next)
    }

    fn statement(
        &mut self,
        statement: &HirStmt,
        continuation: usize,
        break_target: Option<usize>,
        continue_target: Option<usize>,
        handler: Option<GeneratorHandler>,
        cancel_target: usize,
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
                    handler,
                    cancel_target,
                ))
            }
            HirStmt::If(condition, then_body, else_body) => {
                let then_entry = self.sequence(
                    then_body,
                    continuation,
                    break_target,
                    continue_target,
                    handler.clone(),
                    cancel_target,
                )?;
                let else_entry = self.sequence(
                    else_body,
                    continuation,
                    break_target,
                    continue_target,
                    handler.clone(),
                    cancel_target,
                )?;
                Some(self.block(
                    Vec::new(),
                    GeneratorTerm::Branch(condition.clone(), then_entry, else_entry),
                    handler,
                    cancel_target,
                ))
            }
            HirStmt::While(condition, body) => {
                let first_block = self.blocks.len();
                let forwards_control = body.iter().any(|statement| {
                    matches!(statement, HirStmt::Let(name, _, _) if name.starts_with("__thaw_yield_delegate_chunk_"))
                });
                let suspends = body.iter().any(stmt_contains_await);
                let condition_id = self.block(
                    Vec::new(),
                    GeneratorTerm::Done,
                    handler.clone(),
                    cancel_target,
                );
                let body_entry = self.sequence(
                    body,
                    condition_id,
                    Some(continuation),
                    Some(condition_id),
                    handler.clone(),
                    cancel_target,
                )?;
                self.blocks[condition_id].term =
                    GeneratorTerm::Branch(condition.clone(), body_entry, continuation);
                if forwards_control {
                    for block in &mut self.blocks[first_block..] {
                        block.forwards_control = !(suspends
                            && matches!(
                                block.term,
                                GeneratorTerm::Next(next) if next == continuation
                            ));
                    }
                }
                Some(condition_id)
            }
            HirStmt::Break => Some(self.block(
                Vec::new(),
                GeneratorTerm::Next(break_target?),
                handler,
                cancel_target,
            )),
            HirStmt::Continue => Some(self.block(
                Vec::new(),
                GeneratorTerm::Next(continue_target?),
                handler,
                cancel_target,
            )),
            HirStmt::Return(_) => Some(self.block(
                Vec::new(),
                GeneratorTerm::Done,
                handler,
                cancel_target,
            )),
            HirStmt::Try(try_body, catch_name, catch_body) => {
                self.locals.push(HirStmt::Let(
                    catch_name.clone(),
                    HirType::Str,
                    HirExpr::Lit(HirLit::Str(String::new())),
                ));
                let nested_cancel = match self.finalizers.get(catch_name) {
                    Some(finalizer) => self.sequence(
                        finalizer,
                        cancel_target,
                        break_target,
                        continue_target,
                        handler.clone(),
                        cancel_target,
                    )?,
                    None => cancel_target,
                };
                let catch_entry = self.sequence(
                    catch_body,
                    continuation,
                    break_target,
                    continue_target,
                    handler.clone(),
                    nested_cancel,
                )?;
                self.sequence(
                    try_body,
                    continuation,
                    break_target,
                    continue_target,
                    Some(GeneratorHandler {
                        binding: catch_name.clone(),
                        entry: catch_entry,
                    }),
                    nested_cancel,
                )
            }
            HirStmt::BreakDepth(_) | HirStmt::ContinueDepth(_) => None,
            other if generator_emits_value(other, self.values) => {
                let resume = self.block(
                    Vec::new(),
                    GeneratorTerm::Next(continuation),
                    handler.clone(),
                    cancel_target,
                );
                Some(self.block(
                    Vec::new(),
                    GeneratorTerm::Yield(other.clone(), resume),
                    handler,
                    cancel_target,
                ))
            }
            other => Some(self.block(
                vec![other.clone()],
                GeneratorTerm::Next(continuation),
                handler,
                cancel_target,
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
        HirStmt::Expr(HirExpr::Assign(name, value)) if name == values => {
            matches!(value.as_ref(), HirExpr::ArrayConcat(..))
        }
        _ => false,
    }
}

fn generator_statements_emit_value(statements: &[HirStmt], values: &str) -> bool {
    statements.iter().any(|statement| {
        generator_emits_value(statement, values)
            || match statement {
                HirStmt::If(_, then_body, else_body) => {
                    generator_statements_emit_value(then_body, values)
                        || generator_statements_emit_value(else_body, values)
                }
                HirStmt::While(_, body) => generator_statements_emit_value(body, values),
                HirStmt::Try(try_body, _, catch_body) => {
                    generator_statements_emit_value(try_body, values)
                        || generator_statements_emit_value(catch_body, values)
                }
                _ => false,
            }
    })
}

fn generator_placeholder(ty: &HirType) -> Option<HirExpr> {
    Some(match ty {
        HirType::F64 => HirExpr::Lit(HirLit::F64(0.0)),
        HirType::I64 => HirExpr::Lit(HirLit::I64(0)),
        HirType::Bool => HirExpr::Lit(HirLit::Bool(false)),
        HirType::Str => HirExpr::Lit(HirLit::Str(String::new())),
        HirType::Array(_) => HirExpr::ArrayLit(Vec::new()),
        HirType::Tuple(elements) => HirExpr::ArrayLit(
            elements
                .iter()
                .map(generator_placeholder)
                .collect::<Option<Vec<_>>>()?,
        ),
        HirType::Object(fields) => HirExpr::ObjectLit(
            fields
                .iter()
                .map(|(name, ty)| Some((name.clone(), generator_placeholder(ty)?)))
                .collect::<Option<Vec<_>>>()?,
        ),
        HirType::Optional(value) => HirExpr::OptionalNone(value.as_ref().clone()),
        HirType::Nullable(value) => HirExpr::NullableNone(value.as_ref().clone()),
        HirType::Nullish(value) => HirExpr::NullishUndefined(value.as_ref().clone()),
        HirType::Undefined => HirExpr::Lit(HirLit::Undefined),
        HirType::Null => HirExpr::Lit(HirLit::Null),
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn lower_generator_state_machine(
    statements: &[HirStmt],
    finalizers: &HashMap<Symbol, Vec<HirStmt>>,
    values: &str,
    state: &str,
    control: &str,
    error: &str,
    initialized: &str,
    returns: &str,
    return_request: &str,
    pending_return: Option<&str>,
    pending_control: &str,
    suspends: bool,
    forced_return: &str,
) -> Option<(usize, Vec<HirStmt>, Vec<HirStmt>)> {
    let mut machine = GeneratorStateMachine::new(values, finalizers);
    let done = machine.block(Vec::new(), GeneratorTerm::Done, None, 0);
    let entry = machine.sequence(statements, done, None, None, None, done)?;
    let delegates = suspends && machine.blocks.iter().any(|block| block.forwards_control);
    let cancel_dispatch = machine.blocks.iter().enumerate().rev().fold(
        Vec::new(),
        |otherwise, (id, block)| {
            vec![HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Var(state.into())),
                    Box::new(HirExpr::Lit(HirLit::F64(id as f64))),
                ),
                if block.forwards_control {
                    Vec::new()
                } else {
                    vec![
                        HirStmt::Expr(HirExpr::Assign(
                            control.into(),
                            Box::new(HirExpr::Lit(HirLit::I64(0))),
                        )),
                        generator_set_state(state, block.cancel_target),
                    ]
                },
                otherwise,
            )]
        },
    );
    let mut dispatch = Vec::with_capacity(machine.blocks.len());
    for (id, block) in machine.blocks.into_iter().enumerate() {
        let cancel_target = block.cancel_target;
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
                if let Some(pending_return) = pending_return {
                    selected.push(generator_transfer_return(
                        return_request,
                        pending_return,
                    ));
                }
                selected.push(value);
                selected.push(generator_set_state(state, next));
                selected.push(HirStmt::Return(Some(HirExpr::Var(values.into()))));
            }
            GeneratorTerm::Done => {
                if let Some(pending_return) = pending_return {
                    selected.push(generator_transfer_return_unless_completed(
                        pending_return,
                        forced_return,
                        returns,
                    ));
                }
                selected.push(generator_transfer_return_unless_completed(
                    return_request,
                    forced_return,
                    returns,
                ));
                selected.push(HirStmt::Expr(HirExpr::Assign(
                    state.into(),
                    Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                )));
                selected.push(HirStmt::Return(Some(HirExpr::Var(values.into()))));
            }
        }
        if !block.forwards_control {
            selected.insert(
                0,
                generator_control_transition(control, error, values, state, cancel_target),
            );
            let resumed_cancel = vec![
                HirStmt::Expr(HirExpr::Assign(
                    values.into(),
                    Box::new(HirExpr::ArrayLit(Vec::new())),
                )),
                HirStmt::Expr(HirExpr::Assign(
                    control.into(),
                    Box::new(HirExpr::Lit(HirLit::I64(0))),
                )),
                HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_array_shift".into())),
                    vec![HirExpr::Var(pending_control.into())],
                )),
                generator_set_state(state, cancel_target),
            ];
            if delegates {
                selected = vec![HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Gt,
                    Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                        pending_control.into(),
                    )))),
                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                ),
                vec![HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::Var(control.into())),
                        Box::new(HirExpr::Lit(HirLit::I64(0))),
                    ),
                    resumed_cancel,
                    selected.clone(),
                )],
                selected,
                )];
            }
        }
        if let Some(handler) = block.handler {
            let caught = format!("__thaw_generator_caught_{id}");
            selected = vec![HirStmt::Try(
                selected,
                caught.clone(),
                vec![
                    HirStmt::Expr(HirExpr::Assign(
                        handler.binding,
                        Box::new(HirExpr::Var(caught)),
                    )),
                    generator_set_state(state, handler.entry),
                    HirStmt::Continue,
                ],
            )];
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
        vec![
            if delegates { HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Var(control.into())),
                    Box::new(HirExpr::Lit(HirLit::I64(1))),
                ),
                vec![HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                            pending_control.into(),
                        )))),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    ),
                    vec![HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_push".into())),
                        vec![
                            HirExpr::Var(pending_control.into()),
                            HirExpr::Lit(HirLit::Bool(true)),
                        ],
                    ))],
                    Vec::new(),
                )],
                Vec::new(),
            ) } else { HirStmt::Expr(HirExpr::Lit(HirLit::Undefined)) },
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Var(initialized.into())),
                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                ),
                vec![HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::Var(control.into())),
                        Box::new(HirExpr::Lit(HirLit::I64(0))),
                    ),
                    vec![HirStmt::Expr(HirExpr::Assign(
                        initialized.into(),
                        Box::new(HirExpr::Lit(HirLit::Bool(true))),
                    ))],
                    vec![
                        HirStmt::Expr(HirExpr::Assign(
                            state.into(),
                            Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                        )),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(control.into())),
                                Box::new(HirExpr::Lit(HirLit::I64(2))),
                            ),
                            vec![HirStmt::Throw(HirExpr::Var(error.into()))],
                            vec![
                                generator_transfer_return(return_request, forced_return),
                                HirStmt::Return(Some(HirExpr::Var(values.into()))),
                            ],
                        ),
                    ],
                )],
                Vec::new(),
            ),
            pending_return.map(|pending_return| HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Var(control.into())),
                    Box::new(HirExpr::Lit(HirLit::I64(1))),
                ),
                vec![
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::Gt,
                            Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                                pending_return.into(),
                            )))),
                            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        ),
                        vec![HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_shift".into())),
                            vec![HirExpr::Var(pending_return.into())],
                        ))],
                        Vec::new(),
                    ),
                ],
                Vec::new(),
            )).unwrap_or(HirStmt::Expr(HirExpr::Lit(HirLit::Undefined))),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Var(control.into())),
                    Box::new(HirExpr::Lit(HirLit::I64(1))),
                ),
                {
                    let mut cancel = vec![HirStmt::Expr(HirExpr::Assign(
                        values.into(),
                        Box::new(HirExpr::ArrayLit(Vec::new())),
                    ))];
                    cancel.extend(cancel_dispatch);
                    cancel
                },
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(state.into())),
                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                ),
                {
                    let mut done = Vec::new();
                    done.push(HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::Var(control.into())),
                            Box::new(HirExpr::Lit(HirLit::I64(2))),
                        ),
                        vec![HirStmt::Throw(HirExpr::Var(error.into()))],
                        Vec::new(),
                    ));
                    if let Some(pending_return) = pending_return {
                        done.push(generator_transfer_return_unless_completed(
                            pending_return,
                            forced_return,
                            returns,
                        ));
                    }
                    done.push(generator_transfer_return_unless_completed(
                        return_request,
                        forced_return,
                        returns,
                    ));
                    done.push(HirStmt::Return(Some(HirExpr::Var(values.into()))));
                    done
                },
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Gt,
                    Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(values.into())))),
                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                ),
                vec![HirStmt::Return(Some(HirExpr::Var(values.into())))],
                Vec::new(),
            ),
            HirStmt::While(
            HirExpr::Lit(HirLit::Bool(true)),
            dispatch,
            ),
        ],
    ))
}

fn generator_transfer_return(source: &str, destination: &str) -> HirStmt {
    HirStmt::If(
        HirExpr::BinOp(
            BinOp::Gt,
            Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                source.into(),
            )))),
            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
        ),
        vec![HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_array_push".into())),
            vec![
                HirExpr::Var(destination.into()),
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_array_shift".into())),
                    vec![HirExpr::Var(source.into())],
                ),
            ],
        ))],
        Vec::new(),
    )
}

fn generator_transfer_return_unless_completed(
    source: &str,
    destination: &str,
    returns: &str,
) -> HirStmt {
    HirStmt::If(
        HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(returns.into())))),
            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
        ),
        vec![generator_transfer_return(source, destination)],
        Vec::new(),
    )
}

fn generator_control_transition(
    control: &str,
    error: &str,
    values: &str,
    state: &str,
    cancel_target: usize,
) -> HirStmt {
    let cancel = HirStmt::If(
        HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(HirExpr::Var(control.into())),
            Box::new(HirExpr::Lit(HirLit::I64(1))),
        ),
        vec![
            HirStmt::Expr(HirExpr::Assign(
                values.into(),
                Box::new(HirExpr::ArrayLit(Vec::new())),
            )),
            HirStmt::Expr(HirExpr::Assign(
                control.into(),
                Box::new(HirExpr::Lit(HirLit::I64(0))),
            )),
            generator_set_state(state, cancel_target),
            HirStmt::Continue,
        ],
        Vec::new(),
    );
    HirStmt::If(
        HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(HirExpr::Var(control.into())),
            Box::new(HirExpr::Lit(HirLit::I64(2))),
        ),
        vec![
            HirStmt::Expr(HirExpr::Assign(
                control.into(),
                Box::new(HirExpr::Lit(HirLit::I64(0))),
            )),
            HirStmt::Throw(HirExpr::Var(error.into())),
        ],
        vec![cancel],
    )
}

fn generator_set_state(state: &str, next: usize) -> HirStmt {
    HirStmt::Expr(HirExpr::Assign(
        state.into(),
        Box::new(HirExpr::Lit(HirLit::F64(next as f64))),
    ))
}
