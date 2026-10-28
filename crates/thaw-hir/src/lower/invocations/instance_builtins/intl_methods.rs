/// Option keys (and, where a wrong value would silently format differently, the
/// accepted literal values) each native locale method understands; anything else is
/// a compile error rather than a silently different result.
const INTL_COMPARE_OPTIONS: &[&str] = &["sensitivity", "ignorePunctuation", "numeric", "caseFirst"];
const INTL_NUMBER_OPTIONS: &[&str] = &[
    "style", "currency", "currencyDisplay", "minimumIntegerDigits", "minimumFractionDigits",
    "maximumFractionDigits", "minimumSignificantDigits", "maximumSignificantDigits",
    "useGrouping", "signDisplay",
];
const INTL_DATE_OPTIONS: &[&str] = &[
    "weekday", "era", "year", "month", "day", "hour", "minute", "second", "hour12", "hourCycle",
    "timeZone", "timeZoneName", "dateStyle", "timeStyle",
];

fn intl_literal_values(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "signDisplay" => Some(&["auto", "never"]),
        "style" => Some(&["decimal", "percent", "currency"]),
        "timeZoneName" => Some(&["short", "long", "shortOffset", "longOffset"]),
        _ => None,
    }
}

impl<'a> FnLowerer<'a> {
    fn intl_empty_string() -> HirExpr {
        HirExpr::Lit(HirLit::Str(String::new()))
    }

    fn is_undefined_argument(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Ident(ident) if ident.sym == *"undefined" && !self.scope.contains_key("undefined"))
    }

    /// Lowers `(locales?, options?)` into two string operands for the thaw-intl
    /// runtime: the requested locale(s) and `key=value` pairs joined by U+001F.
    fn lower_intl_arguments(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        allowed: &[&str],
        label: &str,
    ) -> Result<(HirExpr, HirExpr), String> {
        if arguments.len() > 2 || arguments.iter().any(|argument| argument.spread.is_some()) {
            return Err(format!("native `{label}` expects at most a locales and an options argument"));
        }
        let locales = match arguments.first() {
            Some(argument) if !self.is_undefined_argument(&argument.expr) => {
                let value = self.lower_expr(&argument.expr)?;
                self.coerce_primitive_to_string(value)?
            }
            _ => Self::intl_empty_string(),
        };
        let mut options = Self::intl_empty_string();
        let Some(argument) = arguments.get(1).filter(|argument| !self.is_undefined_argument(&argument.expr)) else {
            return Ok((locales, options));
        };
        let Expr::Object(literal) = argument.expr.as_ref() else {
            return Err(format!("native `{label}` requires its options to be an object literal ({:?})", format!("{:?}", argument.expr).chars().take(200).collect::<String>()));
        };
        let concat = |left: HirExpr, right: HirExpr| {
            HirExpr::Call(Box::new(HirExpr::Var("__thaw_string_concat".into())), vec![left, right])
        };
        for property in &literal.props {
            let PropOrSpread::Prop(property) = property else {
                return Err(format!("native `{label}` does not support spread options"));
            };
            let Prop::KeyValue(KeyValueProp { key, value }) = property.as_ref() else {
                return Err(format!("native `{label}` supports only `key: value` options"));
            };
            let key = match key {
                PropName::Ident(name) => name.sym.to_string(),
                PropName::Str(name) => name.value.to_string_lossy().into_owned(),
                _ => return Err(format!("native `{label}` supports only plain option names")),
            };
            if self.is_undefined_argument(value) {
                continue;
            }
            if !allowed.contains(&key.as_str()) {
                return Err(format!("native `{label}` does not support the `{key}` option"));
            }
            if let (Some(values), Expr::Lit(Lit::Str(literal))) = (intl_literal_values(&key), value.as_ref()) {
                if !values.iter().any(|accepted| literal.value == *accepted) {
                    return Err(format!("native `{label}` does not support {key}: {:?}", literal.value));
                }
            }
            let value = self.lower_expr(value)?;
            let value = self.coerce_primitive_to_string(value)?;
            let separator = if matches!(&options, HirExpr::Lit(HirLit::Str(text)) if text.is_empty()) { "" } else { "\u{1f}" };
            options = concat(options, HirExpr::Lit(HirLit::Str(format!("{separator}{key}="))));
            options = concat(options, value);
        }
        Ok((locales, options))
    }
}
