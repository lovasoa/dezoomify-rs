use custom_error::custom_error;
use std::str::FromStr;
use regex::Regex;
use lazy_static::lazy_static;
use std::convert::TryInto;
use crate::variable::{Variables, BadVariableError};
use crate::{Vec2d, TileReference};
use serde::{Deserialize, Deserializer, de};


#[derive(Deserialize, Debug)]
struct TileSet {
    variables: Variables,
    url_template: UrlTemplate,
    x_template: IntTemplate,
    y_template: IntTemplate,
}


impl<'a> IntoIterator for &'a TileSet {
    type Item = Result<TileReference, UrlTemplateError>;
    type IntoIter = Box<dyn Iterator<Item=Self::Item> + 'a>;

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
        evalexpr::build_operator_tree(s)
            .map_err(|source| UrlTemplateError::BadExpression { expr: s.into(), source })
            .map(|node| IntTemplate(node))
    }
}

impl<'de> Deserialize<'de> for IntTemplate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

#[derive(Debug)]
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

impl<'de> Deserialize<'de> for UrlTemplate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

#[derive(Debug)]
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

custom_error! {pub UrlTemplateError
    BadExpression{expr:String, source:evalexpr::EvalexprError} = "'{expr}' is not a valid expression: {source}",
    EvalError{source:evalexpr::EvalexprError} = "{source}",
    NumberError{source:std::num::TryFromIntError} = "Number too large: {source}",
    BadVariable{source: BadVariableError} = "Invalid variable: {source}"
}

#[cfg(test)]
mod tests {
    use crate::tile_set::{UrlTemplateError, UrlTemplate, TileSet, IntTemplate};
    use std::str::FromStr;
    use evalexpr::Context;
    use crate::variable::{Variable, Variables};
    use crate::TileReference;

    #[test]
    fn url_template_evaluation() -> Result<(), UrlTemplateError> {
        let tpl = UrlTemplate::from_str("a {{x}} b {{y}} c")?;
        let mut ctx = evalexpr::HashMapContext::new();
        ctx.set_value("x".into(), 0.into());
        ctx.set_value("y".into(), 10.into());
        assert_eq!(tpl.eval(&ctx)?, "a 0 b 10 c");
        Ok(())
    }

    #[test]
    fn tile_iteration() -> Result<(), crate::ZoomError> {
        let ts = TileSet {
            variables: Variables::new(vec![
                Variable::new("x", 0, 1, 1).unwrap(),
                Variable::new("y", 0, 1, 1).unwrap(),
            ]),
            url_template: UrlTemplate::from_str("{{x}}/{{y}}")?,
            x_template: IntTemplate::from_str("x")?,
            y_template: IntTemplate::from_str("y")?,
        };
        let tile_refs: Vec<_> = ts.into_iter().collect::<Result<_, _>>()?;
        let expected: Vec<_> = vec![
            "0 0 0/0",
            "0 1 0/1",
            "1 0 1/0",
            "1 1 1/1",
        ].into_iter().map(TileReference::from_str).collect::<Result<_, _>>()?;
        assert_eq!(expected, tile_refs);
        Ok(())
    }

    #[test]
    fn tileset_from_yaml() {
        let serialized = r#"
variables:
    - name: x
      from: 0
      to: 1
      step: 1
    - name: y
      from: 0
      to: 1
      step: 1
url_template: "{{x}}/{{y}}"
x_template: x
y_template: y
        "#;
        let ts: TileSet = serde_yaml::from_str(serialized).unwrap();
        let tile_refs: Vec<_> = ts.into_iter().collect::<Result<_, _>>().unwrap();
        let expected: Vec<_> = vec![
            "0 0 0/0",
            "0 1 0/1",
            "1 0 1/0",
            "1 1 1/1",
        ].into_iter().map(TileReference::from_str).collect::<Result<_, _>>().unwrap();
        assert_eq!(expected, tile_refs);
    }
}