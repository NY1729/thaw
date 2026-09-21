impl<'a> FnLowerer<'a> {
    /// The Temporal value kinds thaw models. Each is an ordinary object
    /// `{ timestamp, __temporal_<kind> }`: the epoch-millisecond `f64`
    /// `Date` already uses, plus a marker field whose *name* encodes the
    /// kind so the type system (and method dispatch) can tell them apart.
    const TEMPORAL_KINDS: &'static [&'static str] = &[
        "instant",
        "plainDate",
        "plainDateTime",
        "plainTime",
        "plainYearMonth",
        "plainMonthDay",
        "zonedDateTime",
        "duration",
    ];

    fn temporal_kind(ty: &HirType) -> Option<&'static str> {
        let HirType::Object(fields) = ty else {
            return None;
        };
        if !fields.iter().any(|(name, _)| name == "timestamp") {
            return None;
        }
        for (name, _) in fields {
            if let Some(kind) = name.strip_prefix("__temporal_") {
                return Self::TEMPORAL_KINDS.iter().copied().find(|known| *known == kind);
            }
        }
        None
    }

    fn temporal_object(kind: &str, timestamp: HirExpr) -> HirExpr {
        HirExpr::ObjectLit(vec![
            ("timestamp".to_string(), timestamp),
            (format!("__temporal_{kind}"), HirExpr::Lit(HirLit::F64(1.0))),
        ])
    }

    fn temporal_formatter(kind: &str) -> &'static str {
        match kind {
            "plainDate" => "__thaw_temporal_plain_date_to_string",
            "plainDateTime" => "__thaw_temporal_plain_date_time_to_string",
            "plainTime" => "__thaw_temporal_plain_time_to_string",
            "plainYearMonth" => "__thaw_temporal_plain_year_month_to_string",
            "plainMonthDay" => "__thaw_temporal_plain_month_day_to_string",
            "duration" => "__thaw_temporal_duration_to_string",
            _ => "__thaw_temporal_instant_to_string",
        }
    }

    fn temporal_now(kind: &str) -> HirExpr {
        Self::temporal_object(
            kind,
            HirExpr::Call(Box::new(HirExpr::Var("__thaw_temporal_now".into())), Vec::new()),
        )
    }

    fn temporal_timestamp(value: HirExpr, ty: &HirType) -> HirExpr {
        HirExpr::PropAccess(Box::new(value), ty.clone(), "timestamp".to_string())
    }

    /// The epoch-millisecond timestamp an operand represents: a Temporal
    /// value's own `timestamp`, or an ISO 8601 string parsed with the same
    /// parser `Temporal.Instant.from` uses.
    fn temporal_timestamp_operand(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let ty = self.infer_expr_type(&value)?;
        if Self::temporal_kind(&ty).is_some() {
            return Ok(Self::temporal_timestamp(value, &ty));
        }
        if ty == HirType::Str {
            let text = self.coerce_primitive_to_string(value)?;
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_temporal_instant_from_string".into())),
                vec![text],
            ));
        }
        Err(format!(
            "expected a Temporal value or an ISO 8601 string, got {ty:?}"
        ))
    }

    /// A Duration operand as milliseconds: a native `Duration.from` result
    /// (`F64`), an object literal of components, or an ISO 8601 string.
    fn temporal_duration_milliseconds(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let ty = self.infer_expr_type(&value)?;
        if ty == HirType::F64 {
            return Ok(value);
        }
        if Self::temporal_kind(&ty) == Some("duration") {
            return Ok(Self::temporal_timestamp(value, &ty));
        }
        if ty == HirType::Str {
            let text = self.coerce_primitive_to_string(value)?;
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_temporal_duration_from_string".into())),
                vec![text],
            ));
        }
        if let HirExpr::ObjectLit(fields) = &value {
            return Self::duration_fields_milliseconds(fields);
        }
        Err(format!(
            "expected a Duration, an ISO 8601 duration string, or an object of components, got {ty:?}"
        ))
    }

    /// Sums a Duration-like object literal's components into milliseconds.
    fn duration_fields_milliseconds(fields: &[(Symbol, HirExpr)]) -> Result<HirExpr, String> {
        let mut total: Option<HirExpr> = None;
        let mut add = |milliseconds: HirExpr| {
            total = Some(match total.take() {
                Some(existing) => HirExpr::BinOp(BinOp::Add, Box::new(existing), Box::new(milliseconds)),
                None => milliseconds,
            });
        };
        for (name, value) in fields {
            let factor = match name.as_str() {
                "years" => 365.0 * 86_400_000.0,
                "months" => 30.0 * 86_400_000.0,
                "weeks" => 7.0 * 86_400_000.0,
                "days" => 86_400_000.0,
                "hours" => 3_600_000.0,
                "minutes" => 60_000.0,
                "seconds" => 1_000.0,
                "milliseconds" => 1.0,
                "microseconds" => 0.001,
                "nanoseconds" => 0.000001,
                _ => return Err(format!("unknown Duration component `{name}`")),
            };
            add(HirExpr::BinOp(
                BinOp::Mul,
                Box::new(value.clone()),
                Box::new(HirExpr::Lit(HirLit::F64(factor))),
            ));
        }
        total.ok_or_else(|| "an empty Duration has no components".to_string())
    }

    /// `Temporal.Now.<method>()` and `Temporal.<Namespace>.<method>(...)`.
    fn lower_temporal_namespace_call(
        &mut self,
        member: &MemberExpr,
        call: &CallExpr,
    ) -> Result<Option<HirExpr>, String> {
        let Expr::Member(inner) = member.obj.as_ref() else {
            return Ok(None);
        };
        let Expr::Ident(root) = inner.obj.as_ref() else {
            return Ok(None);
        };
        if root.sym != *"Temporal" {
            return Ok(None);
        }
        let MemberProp::Ident(namespace) = &inner.prop else {
            return Ok(None);
        };
        let MemberProp::Ident(method) = &member.prop else {
            return Ok(None);
        };
        let label = format!("Temporal.{}.{}", namespace.sym, method.sym);
        let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;

        if namespace.sym == *"Now" {
            if !arguments.is_empty() {
                return Err(format!("`{label}` expects no arguments"));
            }
            let result = match method.sym.as_ref() {
                "instant" => Self::temporal_now("instant"),
                "plainDateISO" => Self::temporal_now("plainDate"),
                "plainDateTimeISO" => Self::temporal_now("plainDateTime"),
                "plainTimeISO" => Self::temporal_now("plainTime"),
                "zonedDateTimeISO" => Self::temporal_now("zonedDateTime"),
                "timeZoneId" => HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_temporal_time_zone_id".into())),
                    Vec::new(),
                ),
                other => return Err(format!("`Temporal.Now.{other}` is not supported")),
            };
            return self.wrap_call_argument_bindings(result, &bindings).map(Some);
        }

        if namespace.sym == *"Duration" {
            let result = match method.sym.as_ref() {
                "from" => {
                    let [value] = arguments.as_slice() else {
                        return Err(format!("`{label}` expects exactly one argument"));
                    };
                    let milliseconds = self.temporal_duration_milliseconds(value.clone())?;
                    Self::temporal_object("duration", milliseconds)
                }
                other => return Err(format!("`Temporal.Duration.{other}` is not supported")),
            };
            return self.wrap_call_argument_bindings(result, &bindings).map(Some);
        }

        if namespace.sym == *"Instant" {
            let result = match method.sym.as_ref() {
                "from" => {
                    let [value] = arguments.as_slice() else {
                        return Err(format!("`{label}` expects exactly one argument"));
                    };
                    let text = self.coerce_primitive_to_string(value.clone())?;
                    let timestamp = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_instant_from_string".into())),
                        vec![text],
                    );
                    Self::temporal_object("instant", timestamp)
                }
                "fromEpochMilliseconds" => {
                    let [value] = arguments.as_slice() else {
                        return Err(format!("`{label}` expects exactly one argument"));
                    };
                    let value = self.coerce_primitive_to_number(value.clone())?;
                    Self::temporal_object("instant", value)
                }
                "fromEpochSeconds" => {
                    let [value] = arguments.as_slice() else {
                        return Err(format!("`{label}` expects exactly one argument"));
                    };
                    let value = self.coerce_primitive_to_number(value.clone())?;
                    Self::temporal_object(
                        "instant",
                        HirExpr::BinOp(
                            BinOp::Mul,
                            Box::new(value),
                            Box::new(HirExpr::Lit(HirLit::F64(1000.0))),
                        ),
                    )
                }
                "fromEpochNanoseconds" => {
                    let [value] = arguments.as_slice() else {
                        return Err(format!("`{label}` expects exactly one argument"));
                    };
                    // Nanoseconds are approximated by a plain number of
                    // nanoseconds divided down to milliseconds (a fixed-width
                    // `bigint` argument isn't converted here).
                    let value = match self.infer_expr_type(value)? {
                        HirType::F64 => value.clone(),
                        other => {
                            return Err(format!(
                                "`{label}` expects a number of nanoseconds, got {other:?}"
                            ))
                        }
                    };
                    Self::temporal_object(
                        "instant",
                        HirExpr::BinOp(
                            BinOp::Div,
                            Box::new(value),
                            Box::new(HirExpr::Lit(HirLit::F64(1_000_000.0))),
                        ),
                    )
                }
                "compare" => {
                    let [left, right] = arguments.as_slice() else {
                        return Err(format!("`{label}` expects exactly two arguments"));
                    };
                    let left = self.temporal_timestamp_operand(left.clone())?;
                    let right = self.temporal_timestamp_operand(right.clone())?;
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_compare".into())),
                        vec![left, right],
                    )
                }
                other => return Err(format!("`Temporal.Instant.{other}` is not supported")),
            };
            return self.wrap_call_argument_bindings(result, &bindings).map(Some);
        }

        let kind = match namespace.sym.as_ref() {
            "PlainDate" => "plainDate",
            "PlainDateTime" => "plainDateTime",
            "PlainTime" => "plainTime",
            "PlainYearMonth" => "plainYearMonth",
            "PlainMonthDay" => "plainMonthDay",
            "ZonedDateTime" => "zonedDateTime",
            _ => return Ok(None),
        };
        let result = match method.sym.as_ref() {
            "from" => {
                let [value] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one argument"));
                };
                let text = self.coerce_primitive_to_string(value.clone())?;
                let timestamp = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_temporal_instant_from_string".into())),
                    vec![text],
                );
                Self::temporal_object(kind, timestamp)
            }
            "compare" => {
                let [left, right] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly two arguments"));
                };
                let left = self.temporal_timestamp_operand(left.clone())?;
                let right = self.temporal_timestamp_operand(right.clone())?;
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_temporal_compare".into())),
                    vec![left, right],
                )
            }
            other => return Err(format!("`Temporal.{namespace}.{other}` is not supported")),
        };
        self.wrap_call_argument_bindings(result, &bindings).map(Some)
    }

    /// Instance methods on a Temporal-typed receiver. Returns `None` (after
    /// lowering the receiver, which is pure) when the receiver isn't a
    /// Temporal value, so the caller can fall through.
    fn lower_temporal_method(
        &mut self,
        member: &MemberExpr,
        property: &IdentName,
        call: &CallExpr,
    ) -> Result<Option<HirExpr>, String> {
        let receiver = self.lower_expr(&member.obj)?;
        let receiver_type = self.infer_expr_type(&receiver)?;
        let Some(kind) = Self::temporal_kind(&receiver_type) else {
            return Ok(None);
        };
        let timestamp = Self::temporal_timestamp(receiver, &receiver_type);
        let label = format!("Temporal {kind}.{}", property.sym);
        let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
        let result = match property.sym.as_ref() {
            "toString" | "toJSON" => {
                if !arguments.is_empty() {
                    return Err(format!("`{label}` expects no arguments"));
                }
                HirExpr::Call(
                    Box::new(HirExpr::Var(Self::temporal_formatter(kind).into())),
                    vec![timestamp],
                )
            }
            "equals" => {
                let [other] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one argument"));
                };
                let other = self.temporal_timestamp_operand(other.clone())?;
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_compare".into())),
                        vec![timestamp, other],
                    )),
                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                )
            }
            "add" | "subtract" => {
                let [duration] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one argument"));
                };
                let milliseconds = self.temporal_duration_milliseconds(duration.clone())?;
                let delta = if property.sym == *"add" {
                    milliseconds
                } else {
                    HirExpr::BinOp(
                        BinOp::Mul,
                        Box::new(milliseconds),
                        Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                    )
                };
                Self::temporal_object(
                    kind,
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_shift".into())),
                        vec![timestamp, delta],
                    ),
                )
            }
            "since" | "until" => {
                let [other] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one argument"));
                };
                let other = self.temporal_timestamp_operand(other.clone())?;
                // `until` is `other - this`, `since` is `this - other`.
                let (left, right) = if property.sym == *"until" {
                    (other, timestamp)
                } else {
                    (timestamp, other)
                };
                Self::temporal_object(
                    "duration",
                    HirExpr::BinOp(BinOp::Sub, Box::new(left), Box::new(right)),
                )
            }
            "toPlainDate" => Self::temporal_object("plainDate", timestamp),
            "toPlainDateTime" => Self::temporal_object("plainDateTime", timestamp),
            "toPlainTime" => Self::temporal_object("plainTime", timestamp),
            "toZonedDateTimeISO" => Self::temporal_object("zonedDateTime", timestamp),
            _ => return Ok(None),
        };
        self.wrap_call_argument_bindings(result, &bindings).map(Some)
    }

    /// Property reads (`instant.epochMilliseconds`, `date.year`, ...).
    fn lower_temporal_property(
        &mut self,
        member: &MemberExpr,
        property: &IdentName,
        kind: &'static str,
    ) -> Result<Option<HirExpr>, String> {
        let getter = match property.sym.as_ref() {
            "epochMilliseconds" => None,
            "epochSeconds" => Some(("__thaw_temporal_epoch_seconds", false)),
            "month" => Some(("__thaw_date_get_month", false)),
            "year" | "day" | "hour" | "minute" | "second" | "millisecond" => {
                Some((match property.sym.as_ref() {
                    "year" => "__thaw_date_get_full_year",
                    "day" => "__thaw_date_get_date",
                    "hour" => "__thaw_date_get_hours",
                    "minute" => "__thaw_date_get_minutes",
                    "second" => "__thaw_date_get_seconds",
                    _ => "__thaw_date_get_milliseconds",
                }, false))
            }
            "dayOfWeek" => Some(("__thaw_date_get_day", true)),
            _ => return Ok(None),
        };
        let receiver = self.lower_expr(&member.obj)?;
        let receiver_type = self.infer_expr_type(&receiver)?;
        let timestamp = Self::temporal_timestamp(receiver, &receiver_type);
        let _ = kind;
        Ok(Some(match getter {
            None => timestamp,
            Some(("__thaw_temporal_epoch_seconds", _)) => HirExpr::BinOp(
                BinOp::Div,
                Box::new(timestamp),
                Box::new(HirExpr::Lit(HirLit::F64(1000.0))),
            ),
            Some((name, weekday)) => {
                let value = HirExpr::Call(Box::new(HirExpr::Var(name.into())), vec![timestamp]);
                if weekday {
                    // `Date.getDay` is 0=Sunday; Temporal's `dayOfWeek` is
                    // 1=Monday..7=Sunday.
                    HirExpr::Conditional(
                        Box::new(HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(value.clone()),
                            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(7.0))),
                        Box::new(value),
                        HirType::F64,
                    )
                } else if property.sym == *"month" {
                    // `Date.getMonth` is 0-based; Temporal's `month` is 1-based.
                    HirExpr::BinOp(
                        BinOp::Add,
                        Box::new(value),
                        Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                    )
                } else {
                    value
                }
            }
        }))
    }
}
