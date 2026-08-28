use std::convert::TryInto;
use std::str::FromStr;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Deserializer, de};

use custom_error::custom_error;
use evalexpr::DefaultNumericTypes;

use crate::Vec2d;
use crate::template::{Part, Template};

use super::variable::{BadVariableError, Variables};

#[derive(Clone, Deserialize, Debug)]
pub(crate) struct TileSet {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileEntry {
    pub(crate) uri: String,
    pub(crate) position: Vec2d,
}

impl<'a> IntoIterator for &'a TileSet {
    type Item = Result<TileEntry, UrlTemplateError>;
    type IntoIter = Box<dyn Iterator<Item = Self::Item> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(
            (0..self.variables.cardinality().unwrap_or(0)).map(move |index| self.tile_at(index)),
        )
    }
}

impl TileSet {
    pub(crate) fn len(&self) -> Result<u64, BadVariableError> {
        self.variables.cardinality()
    }

    pub(crate) fn tile_at(&self, ordinal: u64) -> Result<TileEntry, UrlTemplateError> {
        let context = self
            .variables
            .context_at(ordinal)
            .map_err(|source| UrlTemplateError::BadVariable { source })?;
        Ok(TileEntry {
            uri: self.url_template.eval(&context)?,
            position: Vec2d {
                x: self.x_template.eval(&context)?,
                y: self.y_template.eval(&context)?,
            },
        })
    }
}

#[derive(Clone, Debug)]
struct IntTemplate(evalexpr::Node);

impl IntTemplate {
    fn eval<C: evalexpr::Context<NumericTypes = DefaultNumericTypes>>(
        &self,
        context: &C,
    ) -> Result<u32, UrlTemplateError> {
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

#[derive(Clone, Debug)]
struct StrTemplate(evalexpr::Node);

impl StrTemplate {
    fn eval<C: evalexpr::Context<NumericTypes = DefaultNumericTypes>>(
        &self,
        context: &C,
    ) -> Result<String, UrlTemplateError> {
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

#[derive(Clone, Debug)]
struct UrlTemplate(Template<StrTemplate>);

impl UrlTemplate {
    fn eval<C: evalexpr::Context<NumericTypes = DefaultNumericTypes>>(
        &self,
        context: &C,
    ) -> Result<String, UrlTemplateError> {
        self.0.try_render(|expression| expression.eval(context))
    }
}

impl FromStr for UrlTemplate {
    type Err = UrlTemplateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        static EXPR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{.*?}}").unwrap());
        static ZERO_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":0(\d+)$").unwrap());
        let mut parts = vec![];
        let mut cursor = 0usize;
        for m in EXPR_RE.find_iter(s) {
            let prev = &s[cursor..m.start()];
            parts.push(Part::literal(prev));
            let mut expression = &s[m.start() + 2..m.end() - 2];
            let mut min_width: usize = 0;
            if let Some(c) = ZERO_RE.captures(expression) {
                expression = &expression[..expression.len() - c[0].len()];
                min_width = c[1].parse().expect("regex matches only numbers");
            }
            parts.push(Part::Hole(expression.parse()?, min_width));
            cursor = m.end();
        }
        parts.push(Part::literal(&s[cursor..]));
        Ok(UrlTemplate(Template(parts)))
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

    use super::super::tile_set::{IntTemplate, TileEntry, TileSet, UrlTemplate, UrlTemplateError};
    use super::super::variable::{VarOrConst, Variables};
    use crate::Vec2d;

    #[test]
    fn url_template_evaluation() -> Result<(), UrlTemplateError> {
        let tpl = UrlTemplate::from_str("a {{x}} b {{y}} c")?;
        let mut ctx = evalexpr::HashMapContext::new();
        ctx.set_value("x".into(), evalexpr::Value::Int(0))?;
        ctx.set_value("y".into(), evalexpr::Value::Int(10))?;
        assert_eq!(tpl.eval(&ctx)?, "a 0 b 10 c");
        Ok(())
    }

    #[test]
    fn url_template_evaluation_leading_zeroes() -> Result<(), UrlTemplateError> {
        let tpl = UrlTemplate::from_str("{{x:03}} {{ x + y/2 :02}}")?;
        let mut ctx = evalexpr::HashMapContext::new();
        ctx.set_value("x".into(), evalexpr::Value::Int(0))?;
        ctx.set_value("y".into(), evalexpr::Value::Int(10))?;
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
        let expected = vec![
            TileEntry {
                uri: "0/0".into(),
                position: Vec2d { x: 0, y: 0 },
            },
            TileEntry {
                uri: "0/1".into(),
                position: Vec2d { x: 0, y: 1 },
            },
            TileEntry {
                uri: "1/0".into(),
                position: Vec2d { x: 1, y: 0 },
            },
            TileEntry {
                uri: "1/1".into(),
                position: Vec2d { x: 1, y: 1 },
            },
        ];
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
        let expected = vec![
            TileEntry {
                uri: "0/0".into(),
                position: Vec2d { x: 0, y: 0 },
            },
            TileEntry {
                uri: "0/100".into(),
                position: Vec2d { x: 0, y: 1 },
            },
            TileEntry {
                uri: "100/0".into(),
                position: Vec2d { x: 1, y: 0 },
            },
            TileEntry {
                uri: "100/100".into(),
                position: Vec2d { x: 1, y: 1 },
            },
        ];
        assert_eq!(expected, tile_refs);
    }
}
