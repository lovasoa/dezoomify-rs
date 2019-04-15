use custom_error::custom_error;
use std::str::FromStr;
use regex::Regex;
use lazy_static::lazy_static;
use std::convert::TryInto;
use evalexpr::Context;
use crate::variable::{Variable, BadVariableError};

struct TileSet {
    variables: Vec<Variable>,
    url_template: UrlTemplate,
    x_template: IntTemplate,
    y_template: IntTemplate,
}

impl TileSet {
    fn iter_contexts() -> impl Iterator<Item=Result<evalexpr::HashMapContext, UrlTemplateError>> {
        let mut ctx = evalexpr::HashMapContext::new();
        std::iter::from_fn(move || {
            ctx.set_value("x".into(), 0.into());
            None
        })
    }
}

struct IntTemplate(evalexpr::Node);

impl IntTemplate {
    fn eval<C: evalexpr::Context>(&self, context: &C) -> Result<u32, UrlTemplateError> {
        let evaluated_int = self.0.eval_int_with_context(context)?;
        Ok(evaluated_int.try_into()?)
    }
}

impl FromStr for IntTemplate {
    type Err = UrlTemplateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        evalexpr::build_operator_tree(s)
            .map_err(|source| UrlTemplateError::BadExpression { expr: s.into(), source })
            .map(|node| IntTemplate(node))
    }
}

struct UrlTemplate {
    parts: Vec<UrlPart>
}

impl UrlTemplate {
    fn eval<C: evalexpr::Context>(&self, context: &C) -> Result<String, UrlTemplateError> {
        self.parts.iter().map(|p| p.eval(context)).collect()
    }
}

impl FromStr for UrlTemplate {
    type Err = UrlTemplateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"\{\{.*?}}").unwrap();
        }
        let mut parts = vec![];
        let mut cursor = 0usize;
        for m in RE.find_iter(s) {
            let prev = &s[cursor..m.start()];
            parts.push(UrlPart::Constant(String::from(prev)));
            let expr_src = &s[m.start() + 2..m.end() - 2];
            parts.push(UrlPart::expression(expr_src)?);
            cursor = m.end();
        }
        parts.push(UrlPart::constant(&s[cursor..]));
        Ok(UrlTemplate { parts })
    }
}

enum UrlPart {
    Constant(String),
    Expression(IntTemplate),
}

impl UrlPart {
    fn constant<T: Into<String>>(s: T) -> UrlPart {
        UrlPart::Constant(s.into())
    }
    fn expression(s: &str) -> Result<UrlPart, UrlTemplateError> {
        s.parse().map(UrlPart::Expression)
    }
    fn eval<C: evalexpr::Context>(&self, context: &C) -> Result<String, UrlTemplateError> {
        match self {
            UrlPart::Constant(s) => Ok(s.clone()),
            UrlPart::Expression(expr) => Ok(format!("{}", expr.eval(context)?))
        }
    }
}

custom_error! {UrlTemplateError
    BadExpression{expr:String, source:evalexpr::EvalexprError} = "'{expr}' is not a valid expression: {source}",
    EvalError{source:evalexpr::EvalexprError} = "{source}",
    NumberError{source:std::num::TryFromIntError} = "Number too large: {source}",
    BadVariable{source: BadVariableError} = "Invalid variable: {source}"
}

#[cfg(test)]
mod tests {
    use crate::tile_logic::{UrlTemplateError, UrlTemplate};
    use std::str::FromStr;
    use evalexpr::Context;

    #[test]
    fn url_template_evaluation() -> Result<(), UrlTemplateError> {
        let tpl = UrlTemplate::from_str("a {{x}} b {{y}} c")?;
        let mut ctx = evalexpr::HashMapContext::new();
        ctx.set_value("x".into(), 0.into());
        ctx.set_value("y".into(), 10.into());
        assert_eq!(tpl.eval(&ctx)?, "a 0 b 10 c");
        Ok(())
    }
}