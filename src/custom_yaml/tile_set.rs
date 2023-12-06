use std::convert::TryInto;
use std::str::FromStr;

use regex::Regex;
use serde::{de, Deserialize, Deserializer};

use custom_error::custom_error;
use lazy_static::lazy_static;

use crate::{TileReference, Vec2d};

use super::variable::{BadVariableError, Variables};

#[derive(Deserialize, Debug)]
pub struct TileSet {
    variables: Variables,
    url_template: UrlTemplate,

    #[serde(default = "default_x_template")]
    x_template: IntTemplate,
    #[serde(default = "default_y_template")]
    y_template: IntTemplate,
}

fn default_x_template() -> IntTemplate {
    "x".parse().unwrap()
}

fn default_y_template() -> IntTemplate {
    "y".parse().unwrap()
}

impl<'a> IntoIterator for &'a TileSet {
    type Item = Result<TileReference, UrlTemplateError>;
    type IntoIter = Box<dyn Iterator<Item = Self::Item> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.variables.iter_contexts().map(move |ctx| {
            let ctx = ctx?;
            Ok(TileReference {
                url: self.url_template.eval(&ctx)?,
                position: Vec2d {
                    x: self.x_template.eval(&ctx)?,
                    y: self.y_template.eval(&ctx)?,
                },
            })
        }))
    }
}

#[derive(Debug)]
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
        Ok(IntTemplate(parse_expr(s)?))
    }
}

#[derive(Debug)]
struct StrTemplate(evalexpr::Node);

impl StrTemplate {
    fn eval<C: evalexpr::Context>(&self, context: &C) -> Result<String, UrlTemplateError> {
        let value = self.0.eval_with_context(context)?;
        value_to_string(value)
    }
}

fn value_to_string(value: evalexpr::Value) -> Result<String, UrlTemplateError> {
    match value {
        evalexpr::Value::String(s) => Ok(s),
        evalexpr::Value::Float(f) => Ok(f.to_string()),
        evalexpr::Value::Int(i) => Ok(i.to_string()),
        evalexpr::Value::Boolean(b) => Ok(b.to_string()),
        evalexpr::Value::Tuple(t) => t.into_iter().map(value_to_string).collect(),
        evalexpr::Value::Empty => Ok(String::new()),
    }
}

impl FromStr for StrTemplate {
    type Err = UrlTemplateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(StrTemplate(parse_expr(s)?))
    }
}

fn parse_expr(s: &str) -> Result<evalexpr::Node, UrlTemplateError> {
    evalexpr::build_operator_tree(s).map_err(|source| UrlTemplateError::BadExpression {
        expr: s.to_string(),
        source,
    })
}

impl<'de> Deserialize<'de> for IntTemplate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

#[derive(Debug)]
struct UrlTemplate {
    parts: Vec<UrlPart>,
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
            static ref EXPR_RE: Regex = Regex::new(r"\{\{.*?}}").unwrap();
            static ref ZERO_RE: Regex = Regex::new(r":0(\d+)$").unwrap();
        }
        let mut parts = vec![];
        let mut cursor = 0usize;
        for m in EXPR_RE.find_iter(s) {
            let prev = &s[cursor..m.start()];
            parts.push(UrlPart::Constant(String::from(prev)));
            let mut expression = &s[m.start() + 2..m.end() - 2];
            let mut min_width: usize = 0;
            if let Some(c) = ZERO_RE.captures(expression) {
                expression = &expression[..expression.len() - c[0].len()];
                min_width = c[1].parse().expect("regex matches only numbers");
            }
            parts.push(UrlPart::expression(expression, min_width)?);
            cursor = m.end();
        }
        parts.push(UrlPart::constant(&s[cursor..]));
        Ok(UrlTemplate { parts })
    }
}

impl<'de> Deserialize<'de> for UrlTemplate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

#[derive(Debug)]
enum UrlPart {
    Constant(String),
    Expression {
        expression: StrTemplate,
        min_width: usize,
    },
}

impl UrlPart {
    fn constant<T: Into<String>>(s: T) -> UrlPart {
        UrlPart::Constant(s.into())
    }
    fn expression(s: &str, min_width: usize) -> Result<UrlPart, UrlTemplateError> {
        Ok(UrlPart::Expression {
            expression: s.parse()?,
            min_width,
        })
    }
    fn eval<C: evalexpr::Context>(&self, context: &C) -> Result<String, UrlTemplateError> {
        match self {
            UrlPart::Constant(s) => Ok(s.clone()),
            UrlPart::Expression {
                expression,
                min_width,
            } => Ok(format!(
                "{:0>width$}",
                expression.eval(context)?,
                width = min_width
            )),
        }
    }
}

custom_error! {pub UrlTemplateError
    BadExpression{expr:String, source:evalexpr::EvalexprError} = "'{expr}' is not a valid expression: {source}",
    EvalError{source:evalexpr::EvalexprError} = "{source}",
    NumberError{source:std::num::TryFromIntError} = "Number too large: {source}",
    BadVariable{source: BadVariableError} = "Invalid variable: {source}"
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use evalexpr::ContextWithMutableVariables;

    use crate::TileReference;

    use super::super::tile_set::{IntTemplate, TileSet, UrlTemplate, UrlTemplateError};
    use super::super::variable::{VarOrConst, Variables};

    #[test]
    fn url_template_evaluation() -> Result<(), UrlTemplateError> {
        let tpl = UrlTemplate::from_str("a {{x}} b {{y}} c")?;
        let mut ctx = evalexpr::HashMapContext::new();
        ctx.set_value("x".into(), 0.into())?;
        ctx.set_value("y".into(), 10.into())?;
        assert_eq!(tpl.eval(&ctx)?, "a 0 b 10 c");
        Ok(())
    }

    #[test]
    fn url_template_evaluation_leading_zeroes() -> Result<(), UrlTemplateError> {
        let tpl = UrlTemplate::from_str("{{x:03}} {{ x + y/2 :02}}")?;
        let mut ctx = evalexpr::HashMapContext::new();
        ctx.set_value("x".into(), 0.into())?;
        ctx.set_value("y".into(), 10.into())?;
        assert_eq!(tpl.eval(&ctx)?, "000 05");
        Ok(())
    }

    #[test]
    fn tile_iteration() {
        let ts = TileSet {
            variables: Variables::new(vec![
                VarOrConst::var("x", 0, 1, 1).unwrap(),
                VarOrConst::var("y", 0, 1, 1).unwrap(),
            ]),
            url_template: UrlTemplate::from_str("{{x}}/{{y}}").unwrap(),
            x_template: IntTemplate::from_str("x").unwrap(),
            y_template: IntTemplate::from_str("y").unwrap(),
        };
        let tile_refs: Vec<_> = ts.into_iter().collect::<Result<_, _>>().unwrap();
        let expected: Vec<_> = vec!["0 0 0/0", "0 1 0/1", "1 0 1/0", "1 1 1/1"]
            .into_iter()
            .map(TileReference::from_str)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(expected, tile_refs);
    }

    #[test]
    fn tileset_from_yaml() {
        let serialized = r#"
variables:
    - name: x
      from: 0
      to: 1
    - name: y
      from: 0
      to: 1
    - name: tile_size
      value: 100
url_template: "{{x*tile_size}}/{{y*tile_size}}"
        "#;
        let ts: TileSet = serde_yaml::from_str(serialized).unwrap();
        let tile_refs: Vec<_> = ts.into_iter().collect::<Result<_, _>>().unwrap();
        let expected: Vec<_> = vec!["0 0 0/0", "0 1 0/100", "1 0 100/0", "1 1 100/100"]
            .into_iter()
            .map(TileReference::from_str)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(expected, tile_refs);
    }
}
