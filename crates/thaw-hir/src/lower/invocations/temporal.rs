impl<'a> FnLowerer<'a> {
    /// The Temporal value kinds thaw models. Each is an ordinary object
    /// `{ timestamp, nanoseconds, __temporal_<kind> }`: the epoch-millisecond
    /// `f64` `Date` already uses, the sub-millisecond nanoseconds
    /// (0..999999), and a marker field whose *name* encodes the kind so the
    /// type system (and method dispatch) can tell them apart.
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

    fn temporal_object(kind: &str, timestamp: HirExpr, nanoseconds: HirExpr) -> HirExpr {
        Self::temporal_object_calendar(
            kind,
            timestamp,
            nanoseconds,
            HirExpr::Lit(HirLit::Str("iso8601".to_string())),
        )
    }

    fn temporal_object_calendar(
        kind: &str,
        timestamp: HirExpr,
        nanoseconds: HirExpr,
        calendar: HirExpr,
    ) -> HirExpr {
        HirExpr::ObjectLit(vec![
            ("timestamp".to_string(), timestamp),
            ("nanoseconds".to_string(), nanoseconds),
            ("calendar".to_string(), calendar),
            (format!("__temporal_{kind}"), HirExpr::Lit(HirLit::F64(1.0))),
        ])
    }

    /// A `ZonedDateTime`: the instant plus the IANA/fixed-offset zone name
    /// it is expressed in (the timezone-aware extra field).
    fn temporal_zoned_object(
        timestamp: HirExpr,
        nanoseconds: HirExpr,
        time_zone: HirExpr,
    ) -> HirExpr {
        Self::temporal_zoned_object_calendar(
            timestamp,
            nanoseconds,
            time_zone,
            HirExpr::Lit(HirLit::Str("iso8601".to_string())),
        )
    }

    fn temporal_zoned_object_calendar(
        timestamp: HirExpr,
        nanoseconds: HirExpr,
        time_zone: HirExpr,
        calendar: HirExpr,
    ) -> HirExpr {
        HirExpr::ObjectLit(vec![
            ("timestamp".to_string(), timestamp),
            ("nanoseconds".to_string(), nanoseconds),
            ("time_zone".to_string(), time_zone),
            ("calendar".to_string(), calendar),
            (
                "__temporal_zonedDateTime".to_string(),
                HirExpr::Lit(HirLit::F64(1.0)),
            ),
        ])
    }

    fn temporal_calendar(value: HirExpr, ty: &HirType) -> HirExpr {
        HirExpr::PropAccess(Box::new(value), ty.clone(), "calendar".to_string())
    }

    /// The UTC timestamp of a date-bearing value's own date, for
    /// calendar-aware field extraction (a `ZonedDateTime`'s local date, or
    /// a `Plain*` value's stored date).
    fn temporal_date_timestamp(receiver: &HirExpr, ty: &HirType, kind: &str) -> HirExpr {
        if kind == "zonedDateTime" {
            HirExpr::Call(
                Box::new(HirExpr::Var(
                    "__thaw_temporal_zoned_plain_timestamp".into(),
                )),
                vec![
                    Self::temporal_timestamp(receiver.clone(), ty),
                    Self::temporal_nanoseconds(receiver.clone(), ty),
                    Self::temporal_zone(receiver.clone(), ty),
                    HirExpr::Lit(HirLit::F64(0.0)),
                ],
            )
        } else {
            Self::temporal_timestamp(receiver.clone(), ty)
        }
    }

    fn temporal_zone(value: HirExpr, ty: &HirType) -> HirExpr {
        HirExpr::PropAccess(Box::new(value), ty.clone(), "time_zone".to_string())
    }

    fn temporal_components(value: HirExpr, ty: &HirType) -> HirExpr {
        HirExpr::PropAccess(Box::new(value), ty.clone(), "components".to_string())
    }

    /// A `Duration`'s components with each value negated or taken absolute
    /// (for `Duration.prototype.negated`/`abs`).
    fn duration_components_transform(
        &mut self,
        components: &HirExpr,
        absolute: bool,
    ) -> Result<HirExpr, String> {
        let entries = Self::DURATION_COMPONENTS
            .iter()
            .map(|name| {
                let value = HirExpr::JsonAsNumber(Box::new(HirExpr::JsonGet(
                    Box::new(components.clone()),
                    (*name).to_string(),
                )));
                let value = if absolute {
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_math_abs".into())),
                        vec![value],
                    )
                } else {
                    HirExpr::BinOp(
                        BinOp::Mul,
                        Box::new(value),
                        Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                    )
                };
                (*name, value)
            })
            .collect();
        self.duration_components_json(entries)
    }

    /// A `Duration`'s total as nanoseconds (`ms * 1e6 + ns`).
    fn duration_total_nanoseconds(milliseconds: &HirExpr, nanoseconds: &HirExpr) -> HirExpr {
        HirExpr::BinOp(
            BinOp::Add,
            Box::new(HirExpr::BinOp(
                BinOp::Mul,
                Box::new(milliseconds.clone()),
                Box::new(HirExpr::Lit(HirLit::F64(1_000_000.0))),
            )),
            Box::new(nanoseconds.clone()),
        )
    }

    /// Calendar identifiers thaw supports (via ICU4X).
    const SUPPORTED_CALENDARS: &'static [&'static str] = &[
        "iso8601",
        "gregory",
        "buddhist",
        "chinese",
        "coptic",
        "dangi",
        "ethiopic",
        "ethioaa",
        "hebrew",
        "indian",
        "islamic",
        "islamic-umalqura",
        "islamic-tbla",
        "islamic-civil",
        "islamic-rgsa",
        "japanese",
        "persian",
        "roc",
    ];

    /// The component names a `Duration` stores, in canonical order.
    const DURATION_COMPONENTS: &'static [&'static str] = &[
        "years",
        "months",
        "weeks",
        "days",
        "hours",
        "minutes",
        "seconds",
        "milliseconds",
        "microseconds",
        "nanoseconds",
    ];

    /// A `Duration` value: the epoch-millisecond/nanosecond total for
    /// arithmetic plus its component breakdown (so an unnormalized input
    /// such as `{ minutes: 90 }` keeps `minutes` 90).
    fn temporal_duration_object(
        milliseconds: HirExpr,
        nanoseconds: HirExpr,
        components: HirExpr,
    ) -> HirExpr {
        HirExpr::ObjectLit(vec![
            ("timestamp".to_string(), milliseconds),
            ("nanoseconds".to_string(), nanoseconds),
            ("components".to_string(), components),
            (
                "__temporal_duration".to_string(),
                HirExpr::Lit(HirLit::F64(1.0)),
            ),
        ])
    }

    /// Wraps a native number as a JSON value and assembles a `components`
    /// JSON object from `(name, number expression)` pairs.
    fn duration_components_json(
        &mut self,
        entries: Vec<(&'static str, HirExpr)>,
    ) -> Result<HirExpr, String> {
        let mut fields = Vec::with_capacity(entries.len());
        for (name, value) in entries {
            let value = self.coerce_primitive_to_number(value)?;
            let value = self.wrap_native_value_as_json(value, HirType::F64)?;
            fields.push((name.to_string(), value));
        }
        Ok(HirExpr::JsonObjectLit(fields, HirType::Json))
    }

    /// The component breakdown of a normalized (total-milliseconds)
    /// duration: years/months/weeks are 0, and the rest are derived by
    /// remainder.
    fn duration_components_from_ms(
        &mut self,
        milliseconds: &HirExpr,
        nanoseconds: &HirExpr,
    ) -> Result<HirExpr, String> {
        let component = |unit: f64| {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_temporal_duration_component".into())),
                vec![milliseconds.clone(), HirExpr::Lit(HirLit::F64(unit))],
            )
        };
        let microseconds = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_math_trunc".into())),
            vec![HirExpr::BinOp(
                BinOp::Div,
                Box::new(nanoseconds.clone()),
                Box::new(HirExpr::Lit(HirLit::F64(1_000.0))),
            )],
        );
        let remaining_nanoseconds = HirExpr::BinOp(
            BinOp::Sub,
            Box::new(nanoseconds.clone()),
            Box::new(HirExpr::BinOp(
                BinOp::Mul,
                Box::new(microseconds.clone()),
                Box::new(HirExpr::Lit(HirLit::F64(1_000.0))),
            )),
        );
        self.duration_components_json(vec![
            ("years", HirExpr::Lit(HirLit::F64(0.0))),
            ("months", HirExpr::Lit(HirLit::F64(0.0))),
            ("weeks", HirExpr::Lit(HirLit::F64(0.0))),
            ("days", component(0.0)),
            ("hours", component(1.0)),
            ("minutes", component(2.0)),
            ("seconds", component(3.0)),
            ("milliseconds", component(4.0)),
            ("microseconds", microseconds),
            ("nanoseconds", remaining_nanoseconds),
        ])
    }

    /// The component breakdown of a `Duration.from` object-literal input
    /// (absent components are 0).
    fn duration_components_from_fields(
        &mut self,
        fields: &[(Symbol, HirExpr)],
    ) -> Result<HirExpr, String> {
        let entries = Self::DURATION_COMPONENTS
            .iter()
            .map(|name| {
                let value = fields
                    .iter()
                    .find(|(field, _)| field.as_str() == *name)
                    .map(|(_, value)| value.clone())
                    .unwrap_or_else(|| HirExpr::Lit(HirLit::F64(0.0)));
                (*name, value)
            })
            .collect();
        self.duration_components_json(entries)
    }

    /// `(milliseconds, sub-millisecond nanoseconds)` that represent a total
    /// nanosecond count (floor division, so the remainder is non-negative).
    fn duration_from_total_nanoseconds(total: HirExpr) -> (HirExpr, HirExpr) {
        let milliseconds = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_math_floor".into())),
            vec![HirExpr::BinOp(
                BinOp::Div,
                Box::new(total.clone()),
                Box::new(HirExpr::Lit(HirLit::F64(1_000_000.0))),
            )],
        );
        let nanoseconds = HirExpr::BinOp(
            BinOp::Sub,
            Box::new(total),
            Box::new(HirExpr::BinOp(
                BinOp::Mul,
                Box::new(milliseconds.clone()),
                Box::new(HirExpr::Lit(HirLit::F64(1_000_000.0))),
            )),
        );
        (milliseconds, nanoseconds)
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
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_temporal_now_nanos".into())),
                Vec::new(),
            ),
        )
    }

    fn temporal_timestamp(value: HirExpr, ty: &HirType) -> HirExpr {
        HirExpr::PropAccess(Box::new(value), ty.clone(), "timestamp".to_string())
    }

    fn temporal_nanoseconds(value: HirExpr, ty: &HirType) -> HirExpr {
        HirExpr::PropAccess(Box::new(value), ty.clone(), "nanoseconds".to_string())
    }

    /// The `(epoch milliseconds, sub-millisecond nanoseconds)` an operand
    /// represents: a Temporal value's own fields, or an ISO 8601 string
    /// parsed the same way `Temporal.Instant.from` is.
    fn temporal_operand(&mut self, value: HirExpr) -> Result<(HirExpr, HirExpr), String> {
        let ty = self.infer_expr_type(&value)?;
        if Self::temporal_kind(&ty).is_some() {
            let milliseconds = Self::temporal_timestamp(value.clone(), &ty);
            let nanoseconds = Self::temporal_nanoseconds(value, &ty);
            return Ok((milliseconds, nanoseconds));
        }
        if ty == HirType::Str {
            let text = self.coerce_primitive_to_string(value)?;
            let milliseconds = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_temporal_instant_from_string".into())),
                vec![text.clone()],
            );
            let nanoseconds = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_temporal_instant_nanos_from_string".into())),
                vec![text],
            );
            return Ok((milliseconds, nanoseconds));
        }
        Err(format!(
            "expected a Temporal value or an ISO 8601 string, got {ty:?}"
        ))
    }

    /// A Duration operand as `(whole milliseconds, sub-millisecond
    /// nanoseconds)`, preserving sub-millisecond precision.
    fn temporal_duration_operand(
        &mut self,
        value: HirExpr,
    ) -> Result<(HirExpr, HirExpr), String> {
        let ty = self.infer_expr_type(&value)?;
        if ty == HirType::F64 {
            return Ok((value, HirExpr::Lit(HirLit::F64(0.0))));
        }
        if Self::temporal_kind(&ty) == Some("duration") {
            let milliseconds = Self::temporal_timestamp(value.clone(), &ty);
            let nanoseconds = Self::temporal_nanoseconds(value, &ty);
            return Ok((milliseconds, nanoseconds));
        }
        if ty == HirType::Str {
            let text = self.coerce_primitive_to_string(value)?;
            let milliseconds = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_temporal_duration_from_string".into())),
                vec![text.clone()],
            );
            let nanoseconds = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_temporal_duration_nanos_from_string".into())),
                vec![text],
            );
            return Ok((milliseconds, nanoseconds));
        }
        if let HirExpr::ObjectLit(fields) = &value {
            return Ok((
                Self::duration_fields_milliseconds(fields)?,
                HirExpr::Lit(HirLit::F64(0.0)),
            ));
        }
        Err(format!(
            "expected a Duration, an ISO 8601 duration string, or an object of components, got {ty:?}"
        ))
    }

    /// A Duration operand's component JSON: a `Duration` value's own
    /// components, a `Duration.from` object literal's fields, or a
    /// normalized breakdown of its total.
    fn duration_components_operand(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let ty = self.infer_expr_type(&value)?;
        if Self::temporal_kind(&ty) == Some("duration") {
            return Ok(Self::temporal_components(value, &ty));
        }
        if let HirExpr::ObjectLit(fields) = &value {
            return self.duration_components_from_fields(fields);
        }
        let (milliseconds, nanoseconds) = self.temporal_duration_operand(value)?;
        self.duration_components_from_ms(&milliseconds, &nanoseconds)
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
            if method.sym == *"zonedDateTimeISO" {
                // Optional time-zone argument (thaw has no system zone, so
                // it defaults to UTC).
                if arguments.len() > 1 {
                    return Err(format!("`{label}` expects at most one argument"));
                }
                let time_zone = match arguments.first() {
                    Some(zone) => self.coerce_primitive_to_string(zone.clone())?,
                    None => HirExpr::Lit(HirLit::Str("UTC".to_string())),
                };
                let result = Self::temporal_zoned_object(
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_now".into())),
                        Vec::new(),
                    ),
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_now_nanos".into())),
                        Vec::new(),
                    ),
                    time_zone,
                );
                return self.wrap_call_argument_bindings(result, &bindings).map(Some);
            }
            if !arguments.is_empty() {
                return Err(format!("`{label}` expects no arguments"));
            }
            let result = match method.sym.as_ref() {
                "instant" => Self::temporal_now("instant"),
                "plainDateISO" => Self::temporal_now("plainDate"),
                "plainDateTimeISO" => Self::temporal_now("plainDateTime"),
                "plainTimeISO" => Self::temporal_now("plainTime"),
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
                    let value_type = self.infer_expr_type(value)?;
                    let (milliseconds, nanoseconds) =
                        self.temporal_duration_operand(value.clone())?;
                    // An object-literal or ISO-string input keeps its own
                    // component breakdown (so `{ minutes: 90 }` and
                    // `"PT90M"` report 90); a native duration is normalized
                    // from its total.
                    let components = match &value {
                        HirExpr::ObjectLit(fields) => {
                            self.duration_components_from_fields(fields)?
                        }
                        _ if value_type == HirType::Str => {
                            let text = self.coerce_primitive_to_string(value.clone())?;
                            HirExpr::Call(
                                Box::new(HirExpr::Var("JSON.parse".into())),
                                vec![HirExpr::Call(
                                    Box::new(HirExpr::Var(
                                        "__thaw_temporal_duration_components_json".into(),
                                    )),
                                    vec![text],
                                )],
                            )
                        }
                        _ => self.duration_components_from_ms(&milliseconds, &nanoseconds)?,
                    };
                    Self::temporal_duration_object(milliseconds, nanoseconds, components)
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
                    let milliseconds = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_instant_from_string".into())),
                        vec![text.clone()],
                    );
                    let nanoseconds = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_instant_nanos_from_string".into())),
                        vec![text],
                    );
                    Self::temporal_object("instant", milliseconds, nanoseconds)
                }
                "fromEpochMilliseconds" => {
                    let [value] = arguments.as_slice() else {
                        return Err(format!("`{label}` expects exactly one argument"));
                    };
                    let value = self.coerce_primitive_to_number(value.clone())?;
                    Self::temporal_object("instant", value, HirExpr::Lit(HirLit::F64(0.0)))
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
                        HirExpr::Lit(HirLit::F64(0.0)),
                    )
                }
                "fromEpochNanoseconds" => {
                    let [value] = arguments.as_slice() else {
                        return Err(format!("`{label}` expects exactly one argument"));
                    };
                    // A plain number of nanoseconds split into milliseconds
                    // and a sub-millisecond remainder (a fixed-width `bigint`
                    // argument isn't converted here).
                    let value = match self.infer_expr_type(value)? {
                        HirType::F64 => value.clone(),
                        other => {
                            return Err(format!(
                                "`{label}` expects a number of nanoseconds, got {other:?}"
                            ))
                        }
                    };
                    let milliseconds = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_math_floor".into())),
                        vec![HirExpr::BinOp(
                            BinOp::Div,
                            Box::new(value.clone()),
                            Box::new(HirExpr::Lit(HirLit::F64(1_000_000.0))),
                        )],
                    );
                    let nanoseconds = HirExpr::BinOp(
                        BinOp::Sub,
                        Box::new(value),
                        Box::new(HirExpr::BinOp(
                            BinOp::Mul,
                            Box::new(milliseconds.clone()),
                            Box::new(HirExpr::Lit(HirLit::F64(1_000_000.0))),
                        )),
                    );
                    Self::temporal_object("instant", milliseconds, nanoseconds)
                }
                "compare" => {
                    let [left, right] = arguments.as_slice() else {
                        return Err(format!("`{label}` expects exactly two arguments"));
                    };
                    let (left_ms, left_ns) = self.temporal_operand(left.clone())?;
                    let (right_ms, right_ns) = self.temporal_operand(right.clone())?;
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_compare".into())),
                        vec![left_ms, left_ns, right_ms, right_ns],
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
            "from" if kind == "zonedDateTime" => {
                let [value] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one argument"));
                };
                let text = self.coerce_primitive_to_string(value.clone())?;
                let milliseconds = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_temporal_zoned_from_string".into())),
                    vec![text.clone()],
                );
                let nanoseconds = HirExpr::Call(
                    Box::new(HirExpr::Var(
                        "__thaw_temporal_zoned_nanos_from_string".into(),
                    )),
                    vec![text.clone()],
                );
                let time_zone = HirExpr::Call(
                    Box::new(HirExpr::Var(
                        "__thaw_temporal_zoned_zone_from_string".into(),
                    )),
                    vec![text.clone()],
                );
                let calendar = HirExpr::Call(
                    Box::new(HirExpr::Var(
                        "__thaw_temporal_calendar_from_string".into(),
                    )),
                    vec![text],
                );
                Self::temporal_zoned_object_calendar(
                    milliseconds,
                    nanoseconds,
                    time_zone,
                    calendar,
                )
            }
            "from" => {
                let [value] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one argument"));
                };
                let text = self.coerce_primitive_to_string(value.clone())?;
                // `PlainTime` has no date component, so it parses a
                // time-of-day (or the time part of a full date-time).
                let (milliseconds_parser, nanoseconds_parser) = if kind == "plainTime" {
                    (
                        "__thaw_temporal_plain_time_from_string",
                        "__thaw_temporal_plain_time_nanos_from_string",
                    )
                } else {
                    (
                        "__thaw_temporal_instant_from_string",
                        "__thaw_temporal_instant_nanos_from_string",
                    )
                };
                let milliseconds = HirExpr::Call(
                    Box::new(HirExpr::Var(milliseconds_parser.into())),
                    vec![text.clone()],
                );
                let nanoseconds = HirExpr::Call(
                    Box::new(HirExpr::Var(nanoseconds_parser.into())),
                    vec![text.clone()],
                );
                let calendar = HirExpr::Call(
                    Box::new(HirExpr::Var(
                        "__thaw_temporal_calendar_from_string".into(),
                    )),
                    vec![text],
                );
                Self::temporal_object_calendar(kind, milliseconds, nanoseconds, calendar)
            }
            "compare" => {
                let [left, right] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly two arguments"));
                };
                let (left_ms, left_ns) = self.temporal_operand(left.clone())?;
                let (right_ms, right_ns) = self.temporal_operand(right.clone())?;
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_temporal_compare".into())),
                    vec![left_ms, left_ns, right_ms, right_ns],
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
        let timestamp = Self::temporal_timestamp(receiver.clone(), &receiver_type);
        let nanoseconds = Self::temporal_nanoseconds(receiver.clone(), &receiver_type);
        let time_zone = (kind == "zonedDateTime")
            .then(|| Self::temporal_zone(receiver.clone(), &receiver_type));
        let label = format!("Temporal {kind}.{}", property.sym);
        let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
        let result = match property.sym.as_ref() {
            "toString" | "toJSON" => {
                if !arguments.is_empty() {
                    return Err(format!("`{label}` expects no arguments"));
                }
                if kind == "duration" {
                    // Format from the stored components, so an unnormalized
                    // duration prints unnormalized.
                    return Ok(Some(HirExpr::Call(
                        Box::new(HirExpr::Var(
                            "__thaw_temporal_duration_to_string_components".into(),
                        )),
                        vec![Self::temporal_components(receiver, &receiver_type)],
                    )));
                }
                match &time_zone {
                    Some(zone) => HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_zoned_to_string".into())),
                        vec![timestamp, nanoseconds, zone.clone()],
                    ),
                    None => HirExpr::Call(
                        Box::new(HirExpr::Var(Self::temporal_formatter(kind).into())),
                        vec![timestamp, nanoseconds],
                    ),
                }
            }
            "equals" => {
                let [other] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one argument"));
                };
                let (other_ms, other_ns) = self.temporal_operand(other.clone())?;
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_compare".into())),
                        vec![timestamp, nanoseconds, other_ms, other_ns],
                    )),
                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                )
            }
            "add" | "subtract" => {
                let [duration] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one argument"));
                };
                let (delta_ms, delta_ns) =
                    self.temporal_duration_operand(duration.clone())?;
                let negate = |value: HirExpr| {
                    HirExpr::BinOp(
                        BinOp::Mul,
                        Box::new(value),
                        Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                    )
                };
                let (delta_ms, delta_ns) = if property.sym == *"add" {
                    (delta_ms, delta_ns)
                } else {
                    (negate(delta_ms), negate(delta_ns))
                };
                // Add the nanosecond parts, carrying whole milliseconds into
                // the timestamp so sub-millisecond durations aren't lost.
                let nanosecond_sum =
                    HirExpr::BinOp(BinOp::Add, Box::new(nanoseconds), Box::new(delta_ns));
                // Floor (not truncate) so a negative sum borrows a whole
                // millisecond and leaves a non-negative remainder.
                let carry = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_math_floor".into())),
                    vec![HirExpr::BinOp(
                        BinOp::Div,
                        Box::new(nanosecond_sum.clone()),
                        Box::new(HirExpr::Lit(HirLit::F64(1_000_000.0))),
                    )],
                );
                let result_nanoseconds = HirExpr::BinOp(
                    BinOp::Sub,
                    Box::new(nanosecond_sum),
                    Box::new(HirExpr::BinOp(
                        BinOp::Mul,
                        Box::new(carry.clone()),
                        Box::new(HirExpr::Lit(HirLit::F64(1_000_000.0))),
                    )),
                );
                let result_milliseconds = HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(HirExpr::BinOp(
                        BinOp::Add,
                        Box::new(timestamp),
                        Box::new(delta_ms),
                    )),
                    Box::new(carry),
                );
                if kind == "duration" {
                    // `Duration.add`/`subtract` are component-wise, so an
                    // unnormalized input stays unnormalized.
                    let left_components =
                        Self::temporal_components(receiver.clone(), &receiver_type);
                    let right_components =
                        self.duration_components_operand(duration.clone())?;
                    let subtract = property.sym == *"subtract";
                    let entries = Self::DURATION_COMPONENTS
                        .iter()
                        .map(|name| {
                            let left = HirExpr::JsonAsNumber(Box::new(HirExpr::JsonGet(
                                Box::new(left_components.clone()),
                                (*name).to_string(),
                            )));
                            let right = HirExpr::JsonAsNumber(Box::new(HirExpr::JsonGet(
                                Box::new(right_components.clone()),
                                (*name).to_string(),
                            )));
                            let value = HirExpr::BinOp(
                                if subtract { BinOp::Sub } else { BinOp::Add },
                                Box::new(left),
                                Box::new(right),
                            );
                            (*name, value)
                        })
                        .collect();
                    let components = self.duration_components_json(entries)?;
                    Self::temporal_duration_object(
                        result_milliseconds,
                        result_nanoseconds,
                        components,
                    )
                } else {
                    match &time_zone {
                        Some(zone) => {
                            let calendar =
                                Self::temporal_calendar(receiver.clone(), &receiver_type);
                            Self::temporal_zoned_object_calendar(
                                result_milliseconds,
                                result_nanoseconds,
                                zone.clone(),
                                calendar,
                            )
                        }
                        None => {
                            let calendar =
                                Self::temporal_calendar(receiver, &receiver_type);
                            Self::temporal_object_calendar(
                                kind,
                                result_milliseconds,
                                result_nanoseconds,
                                calendar,
                            )
                        }
                    }
                }
            }
            "since" | "until" => {
                let [other] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one argument"));
                };
                let (other_ms, _) = self.temporal_operand(other.clone())?;
                // `until` is `other - this`, `since` is `this - other`.
                let (left, right) = if property.sym == *"until" {
                    (other_ms, timestamp)
                } else {
                    (timestamp, other_ms)
                };
                let milliseconds =
                    HirExpr::BinOp(BinOp::Sub, Box::new(left), Box::new(right));
                let nanoseconds = HirExpr::Lit(HirLit::F64(0.0));
                let components =
                    self.duration_components_from_ms(&milliseconds, &nanoseconds)?;
                Self::temporal_duration_object(milliseconds, nanoseconds, components)
            }
            "toPlainDate" | "toPlainDateTime" | "toPlainTime" => {
                let plain_kind = match property.sym.as_ref() {
                    "toPlainDate" => "plainDate",
                    "toPlainTime" => "plainTime",
                    _ => "plainDateTime",
                };
                match &time_zone {
                    Some(zone) => {
                        // The plain value is the local wall clock in the
                        // zone, stored as a UTC timestamp of the same
                        // fields so the shared formatter renders it.
                        let mode = match property.sym.as_ref() {
                            "toPlainDate" => 0.0,
                            "toPlainTime" => 1.0,
                            _ => 2.0,
                        };
                        let local = HirExpr::Call(
                            Box::new(HirExpr::Var(
                                "__thaw_temporal_zoned_plain_timestamp".into(),
                            )),
                            vec![
                                timestamp,
                                nanoseconds.clone(),
                                zone.clone(),
                                HirExpr::Lit(HirLit::F64(mode)),
                            ],
                        );
                        let calendar =
                            Self::temporal_calendar(receiver.clone(), &receiver_type);
                        Self::temporal_object_calendar(
                            plain_kind,
                            local,
                            nanoseconds,
                            calendar,
                        )
                    }
                    None => {
                        let calendar = Self::temporal_calendar(receiver, &receiver_type);
                        Self::temporal_object_calendar(
                            plain_kind,
                            timestamp,
                            nanoseconds,
                            calendar,
                        )
                    }
                }
            }
            "toInstant" => {
                Self::temporal_object("instant", timestamp, nanoseconds)
            }
            "withCalendar" => {
                let [calendar] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one calendar"));
                };
                let calendar_name = match calendar {
                    HirExpr::Lit(HirLit::Str(calendar)) => calendar.to_ascii_lowercase(),
                    _ => {
                        return Err(format!("`{label}` requires a string-literal calendar"));
                    }
                };
                if !Self::SUPPORTED_CALENDARS.contains(&calendar_name.as_str()) {
                    return Err(format!("unknown Temporal calendar `{calendar_name}`"));
                }
                let calendar_literal = HirExpr::Lit(HirLit::Str(calendar_name));
                match &time_zone {
                    Some(zone) => Self::temporal_zoned_object_calendar(
                        timestamp,
                        nanoseconds,
                        zone.clone(),
                        calendar_literal,
                    ),
                    None => Self::temporal_object_calendar(
                        kind,
                        timestamp,
                        nanoseconds,
                        calendar_literal,
                    ),
                }
            }
            "withTimeZone" => {
                let [zone] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one time zone"));
                };
                let zone = self.coerce_primitive_to_string(zone.clone())?;
                let calendar = Self::temporal_calendar(receiver.clone(), &receiver_type);
                Self::temporal_zoned_object_calendar(timestamp, nanoseconds, zone, calendar)
            }
            "toZonedDateTimeISO" => {
                let zone = match arguments.first() {
                    Some(zone) => self.coerce_primitive_to_string(zone.clone())?,
                    None => match &time_zone {
                        Some(zone) => zone.clone(),
                        None => {
                            return Err(format!("`{label}` requires a time zone"));
                        }
                    },
                };
                let calendar = Self::temporal_calendar(receiver.clone(), &receiver_type);
                Self::temporal_zoned_object_calendar(timestamp, nanoseconds, zone, calendar)
            }
            "abs" | "negated" if kind == "duration" => {
                let total = Self::duration_total_nanoseconds(&timestamp, &nanoseconds);
                let total = if property.sym == *"abs" {
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_math_abs".into())),
                        vec![total],
                    )
                } else {
                    HirExpr::BinOp(
                        BinOp::Mul,
                        Box::new(total),
                        Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                    )
                };
                let (milliseconds, nanoseconds) =
                    Self::duration_from_total_nanoseconds(total);
                // The components are transformed the same way, so an
                // unnormalized input stays unnormalized.
                let components = self.duration_components_transform(
                    &Self::temporal_components(receiver, &receiver_type),
                    property.sym == *"abs",
                )?;
                Self::temporal_duration_object(milliseconds, nanoseconds, components)
            }
            "total" if kind == "duration" => {
                let [unit] = arguments.as_slice() else {
                    return Err(format!("`{label}` expects exactly one unit argument"));
                };
                let factor = Self::duration_unit_factor(&Self::duration_unit(unit)?)?;
                HirExpr::BinOp(
                    BinOp::Div,
                    Box::new(timestamp),
                    Box::new(HirExpr::Lit(HirLit::F64(factor))),
                )
            }
            _ => return Ok(None),
        };
        self.wrap_call_argument_bindings(result, &bindings).map(Some)
    }

    /// The unit name a `Duration.total`/`Duration.round` argument names: a
    /// bare string, or an object literal's `unit` field.
    fn duration_unit(argument: &HirExpr) -> Result<String, String> {
        if let HirExpr::Lit(HirLit::Str(unit)) = argument {
            return Ok(unit.clone());
        }
        if let HirExpr::ObjectLit(fields) = argument {
            for (name, value) in fields {
                if name == "unit" {
                    if let HirExpr::Lit(HirLit::Str(unit)) = value {
                        return Ok(unit.clone());
                    }
                }
            }
        }
        Err("a Duration unit must be a string literal or `{ unit: \"...\" }`".into())
    }

    fn duration_unit_factor(unit: &str) -> Result<f64, String> {
        // Temporal accepts both the singular and plural unit spellings.
        Ok(match unit {
            "year" | "years" => 365.0 * 86_400_000.0,
            "month" | "months" => 30.0 * 86_400_000.0,
            "week" | "weeks" => 7.0 * 86_400_000.0,
            "day" | "days" => 86_400_000.0,
            "hour" | "hours" => 3_600_000.0,
            "minute" | "minutes" => 60_000.0,
            "second" | "seconds" => 1_000.0,
            "millisecond" | "milliseconds" => 1.0,
            "microsecond" | "microseconds" => 0.001,
            "nanosecond" | "nanoseconds" => 0.000001,
            other => return Err(format!("unknown Duration unit `{other}`")),
        })
    }

    /// Property reads (`instant.epochMilliseconds`, `date.year`, ...).
    fn lower_temporal_property(
        &mut self,
        member: &MemberExpr,
        property: &IdentName,
        kind: &'static str,
    ) -> Result<Option<HirExpr>, String> {
        if property.sym == *"calendarId" {
            let receiver = self.lower_expr(&member.obj)?;
            let receiver_type = self.infer_expr_type(&receiver)?;
            return Ok(Some(Self::temporal_calendar(receiver, &receiver_type)));
        }
        // A `Duration`'s components are approximated from its total
        // milliseconds (a duration isn't stored component-wise, so an
        // unnormalized `{ minutes: 90 }` reports `hours` 1 rather than 0).
        if kind == "duration" {
            if matches!(property.sym.as_ref(), "sign" | "blank") {
                let receiver = self.lower_expr(&member.obj)?;
                let receiver_type = self.infer_expr_type(&receiver)?;
                let milliseconds =
                    Self::temporal_timestamp(receiver.clone(), &receiver_type);
                let nanoseconds = Self::temporal_nanoseconds(receiver, &receiver_type);
                let total = Self::duration_total_nanoseconds(&milliseconds, &nanoseconds);
                if property.sym == *"blank" {
                    return Ok(Some(HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(total),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    )));
                }
                return Ok(Some(HirExpr::Conditional(
                    Box::new(HirExpr::BinOp(
                        BinOp::Gt,
                        Box::new(total.clone()),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    )),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                    Box::new(HirExpr::Conditional(
                        Box::new(HirExpr::BinOp(
                            BinOp::Lt,
                            Box::new(total),
                            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        HirType::F64,
                    )),
                    HirType::F64,
                )));
            }
            if Self::DURATION_COMPONENTS.contains(&property.sym.as_ref()) {
                let receiver = self.lower_expr(&member.obj)?;
                let receiver_type = self.infer_expr_type(&receiver)?;
                let components = Self::temporal_components(receiver, &receiver_type);
                return Ok(Some(HirExpr::JsonAsNumber(Box::new(HirExpr::JsonGet(
                    Box::new(components),
                    property.sym.to_string(),
                )))));
            }
            return Ok(None);
        }
        // `epochNanoseconds` is a BigInt of the full nanosecond count,
        // built from the runtime's decimal string (an `i64` couldn't hold
        // the full range, and `f64` would lose precision).
        if property.sym == *"epochNanoseconds" {
            let receiver = self.lower_expr(&member.obj)?;
            let receiver_type = self.infer_expr_type(&receiver)?;
            let milliseconds = Self::temporal_timestamp(receiver.clone(), &receiver_type);
            let nanoseconds = Self::temporal_nanoseconds(receiver, &receiver_type);
            let digits = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_temporal_epoch_nanoseconds".into())),
                vec![milliseconds, nanoseconds],
            );
            let constructor = HirExpr::Call(
                Box::new(HirExpr::Var("getDynamicValue".into())),
                vec![HirExpr::Lit(HirLit::Str("BigInt".into()))],
            );
            let arguments =
                self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![digits]))?;
            return Ok(Some(HirExpr::Call(
                Box::new(HirExpr::Var("callDynamicValueHandle".into())),
                vec![constructor, arguments],
            )));
        }
        // Calendar-aware date fields, shared by `PlainDate`/`PlainDateTime`/
        // `PlainYearMonth` (and a `ZonedDateTime`'s local date). ICU4X
        // handles the ISO calendar too, so this is used for every calendar.
        let calendar_field = match property.sym.as_ref() {
            "year" => Some(0.0),
            "month" => Some(1.0),
            "day" => Some(2.0),
            "dayOfWeek" => Some(3.0),
            "dayOfYear" => Some(4.0),
            "daysInMonth" => Some(5.0),
            "daysInYear" => Some(6.0),
            "monthsInYear" => Some(7.0),
            "inLeapYear" => Some(8.0),
            "eraYear" => Some(9.0),
            _ => None,
        };
        if property.sym == *"monthCode"
            || property.sym == *"era"
            || calendar_field.is_some()
        {
            let receiver = self.lower_expr(&member.obj)?;
            let receiver_type = self.infer_expr_type(&receiver)?;
            let date_timestamp =
                Self::temporal_date_timestamp(&receiver, &receiver_type, kind);
            let calendar = Self::temporal_calendar(receiver, &receiver_type);
            if property.sym == *"monthCode" {
                return Ok(Some(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_temporal_calendar_month_code".into())),
                    vec![date_timestamp, calendar],
                )));
            }
            if property.sym == *"era" {
                return Ok(Some(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_temporal_calendar_era".into())),
                    vec![date_timestamp, calendar],
                )));
            }
            let value = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_temporal_calendar_field".into())),
                vec![
                    date_timestamp,
                    calendar,
                    HirExpr::Lit(HirLit::F64(calendar_field.unwrap())),
                ],
            );
            if property.sym == *"inLeapYear" {
                // `inLeapYear` is a boolean, not the raw 1/0.
                return Ok(Some(HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(value),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    )),
                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                )));
            }
            return Ok(Some(value));
        }
        // A `ZonedDateTime`'s zone-dependent reads: `timeZoneId`, `offset`,
        // and the calendar/time fields in its own zone.
        if kind == "zonedDateTime" {
            if property.sym == *"timeZoneId" {
                let receiver = self.lower_expr(&member.obj)?;
                let receiver_type = self.infer_expr_type(&receiver)?;
                return Ok(Some(Self::temporal_zone(receiver, &receiver_type)));
            }
            let field = match property.sym.as_ref() {
                "hour" => Some(3.0),
                "minute" => Some(4.0),
                "second" => Some(5.0),
                "millisecond" => Some(6.0),
                "offset" => None,
                _ => None,
            };
            if property.sym == *"offset" || field.is_some() {
                let receiver = self.lower_expr(&member.obj)?;
                let receiver_type = self.infer_expr_type(&receiver)?;
                let milliseconds = Self::temporal_timestamp(receiver.clone(), &receiver_type);
                let nanoseconds = Self::temporal_nanoseconds(receiver.clone(), &receiver_type);
                let zone = Self::temporal_zone(receiver, &receiver_type);
                if property.sym == *"offset" {
                    return Ok(Some(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_temporal_zoned_offset".into())),
                        vec![milliseconds, nanoseconds, zone],
                    )));
                }
                return Ok(Some(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_temporal_zoned_field".into())),
                    vec![
                        milliseconds,
                        nanoseconds,
                        zone,
                        HirExpr::Lit(HirLit::F64(field.unwrap())),
                    ],
                )));
            }
        }
        let getter = match property.sym.as_ref() {
            "epochMilliseconds" => None,
            "epochSeconds" => Some(("__thaw_temporal_epoch_seconds", false)),
            "hour" | "minute" | "second" | "millisecond" => {
                Some((match property.sym.as_ref() {
                    "hour" => "__thaw_date_get_hours",
                    "minute" => "__thaw_date_get_minutes",
                    "second" => "__thaw_date_get_seconds",
                    _ => "__thaw_date_get_milliseconds",
                }, false))
            }
            _ => return Ok(None),
        };
        let receiver = self.lower_expr(&member.obj)?;
        let receiver_type = self.infer_expr_type(&receiver)?;
        let timestamp = Self::temporal_timestamp(receiver.clone(), &receiver_type);
        let nanoseconds = Self::temporal_nanoseconds(receiver, &receiver_type);
        let _ = kind;
        Ok(Some(match getter {
            None => timestamp,
            Some(("__thaw_temporal_epoch_seconds", _)) => HirExpr::BinOp(
                BinOp::Div,
                Box::new(HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(timestamp),
                    Box::new(HirExpr::BinOp(
                        BinOp::Div,
                        Box::new(nanoseconds),
                        Box::new(HirExpr::Lit(HirLit::F64(1_000_000.0))),
                    )),
                )),
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
