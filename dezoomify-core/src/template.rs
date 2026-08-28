use std::convert::Infallible;
use std::fmt::{Display, Write as _};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Template<H>(pub(crate) Vec<Part<H>>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Part<H> {
    Literal(Arc<str>),
    Hole(H, usize),
}

impl<H> Part<H> {
    pub(crate) fn literal(value: impl Into<Arc<str>>) -> Self {
        Self::Literal(value.into())
    }
}

impl<H> Template<H> {
    pub(crate) fn render<D: Display>(&self, mut value: impl FnMut(&H) -> D) -> String {
        self.try_render(|hole| Ok::<_, Infallible>(value(hole)))
            .unwrap_or_else(|never| match never {})
    }

    pub(crate) fn try_render<D: Display, E>(
        &self,
        mut value: impl FnMut(&H) -> Result<D, E>,
    ) -> Result<String, E> {
        let mut output = String::new();
        for part in &self.0 {
            match part {
                Part::Literal(literal) => output.push_str(literal),
                Part::Hole(hole, padding) => {
                    let value = value(hole)?;
                    push_padded(&mut output, value, *padding);
                }
            }
        }
        Ok(output)
    }
}

pub(crate) fn push_padded(output: &mut String, value: impl Display, padding: usize) {
    write!(output, "{value:0>padding$}").expect("writing to String cannot fail");
}
