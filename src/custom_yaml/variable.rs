use std::sync::LazyLock;

use evalexpr::{ContextWithMutableVariables, DefaultNumericTypes, HashMapContext};
use regex::Regex;
use serde::Deserialize;

use custom_error::custom_error;

use self::VarOrConst::Var;

#[derive(Clone, Debug, Deserialize)]
pub struct Variable {
    name: String,
    from: i64,
    to: i64,
    #[serde(default = "default_step")]
    step: i64,
}

fn default_step() -> i64 {
    1
}

impl Variable {
    fn check(&self) -> Result<(), BadVariableError> {
        static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\w+$").unwrap());
        if !RE.is_match(&self.name) {
            return Err(BadVariableError::BadName {
                name: self.name.clone(),
            });
        }
        if self.step == 0 {
            return Err(BadVariableError::Infinite {
                name: self.name.clone(),
            });
        }
        let steps = (i128::from(self.to) - i128::from(self.from)) / i128::from(self.step);
        if steps < 0 {
            return Err(BadVariableError::Infinite {
                name: self.name.clone(),
            });
        } else if steps > i128::from(u32::MAX) {
            return Err(BadVariableError::TooManyValues {
                name: self.name.clone(),
                steps: i64::MAX,
            });
        }
        Ok(())
    }

    fn len(&self) -> Result<u64, BadVariableError> {
        self.check()?;
        u64::try_from((i128::from(self.to) - i128::from(self.from)) / i128::from(self.step) + 1)
            .map_err(|_| BadVariableError::TooManyValues {
                name: self.name.clone(),
                steps: i64::MAX,
            })
    }

    fn value_at(&self, index: u64) -> i64 {
        debug_assert!(index < self.len().expect("validated variable"));
        i64::try_from(i128::from(self.from) + i128::from(self.step) * i128::from(index))
            .expect("validated variable range fits in i64")
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone)]
pub struct VariableIterator {
    from: i64,
    to: i64,
    step: i64,
    current: i64,
}

impl<'a> VariableIterator {
    fn in_range(&'a self) -> bool {
        let i = self.current;
        (self.from <= i && i <= self.to) || (self.to <= i && i <= self.from)
    }
}

impl Iterator for VariableIterator {
    type Item = i64;

    fn next(&mut self) -> Option<Self::Item> {
        if self.in_range() {
            let current = self.current;
            self.current += self.step;
            Some(current)
        } else {
            None
        }
    }
}

impl IntoIterator for &Variable {
    type Item = i64;
    type IntoIter = VariableIterator;

    fn into_iter(self) -> Self::IntoIter {
        VariableIterator {
            from: self.from,
            to: self.to,
            step: self.step,
            current: self.from,
        }
    }
}

/// Represents a Variable that can have only a single value
#[derive(Deserialize, Clone, Debug)]
pub struct Constant {
    name: String,
    value: i64,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum VarOrConst {
    Var(Variable),
    Const(Constant),
}

impl VarOrConst {
    pub fn var(name: &str, from: i64, to: i64, step: i64) -> Result<VarOrConst, BadVariableError> {
        let var = Variable {
            name: name.to_string(),
            from,
            to,
            step,
        };
        var.check().and(Ok(Var(var)))
    }
    pub fn name(&self) -> &str {
        match self {
            VarOrConst::Var(v) => v.name(),
            VarOrConst::Const(c) => &c.name,
        }
    }
}

impl IntoIterator for &VarOrConst {
    type Item = i64;
    type IntoIter = VariableIterator;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            VarOrConst::Var(v) => v.into_iter(),
            VarOrConst::Const(c) => VariableIterator {
                from: c.value,
                to: c.value,
                current: c.value,
                step: 1,
            },
        }
    }
}

#[derive(Clone, Deserialize, Debug)]
pub struct Variables(Vec<VarOrConst>);

impl Variables {
    #[cfg(test)]
    pub fn new(vars: Vec<VarOrConst>) -> Variables {
        Variables(vars)
    }

    #[cfg(test)]
    fn iter_contexts(
        &self,
    ) -> impl Iterator<Item = Result<HashMapContext<DefaultNumericTypes>, BadVariableError>> + '_
    {
        let count = self.cardinality();
        (0..count.unwrap_or(0)).map(move |index| self.context_at(index))
    }

    pub(crate) fn cardinality(&self) -> Result<u64, BadVariableError> {
        self.0.iter().try_fold(1u64, |cardinality, variable| {
            let values = match variable {
                VarOrConst::Var(variable) => variable.len()?,
                VarOrConst::Const(_) => 1,
            };
            cardinality
                .checked_mul(values)
                .ok_or_else(|| BadVariableError::TooManyValues {
                    name: "combined variables".into(),
                    steps: i64::MAX,
                })
        })
    }

    pub(crate) fn context_at(
        &self,
        mut index: u64,
    ) -> Result<HashMapContext<DefaultNumericTypes>, BadVariableError> {
        let cardinality = self.cardinality()?;
        if index >= cardinality {
            return Err(BadVariableError::TooManyValues {
                name: "variable index".into(),
                steps: i64::MAX,
            });
        }
        let mut ctx = build_context();
        let mut values = Vec::with_capacity(self.0.len());
        for variable in self.0.iter().rev() {
            let radix = match variable {
                VarOrConst::Var(variable) => variable.len()?,
                VarOrConst::Const(_) => 1,
            };
            let value_index = index % radix;
            index /= radix;
            values.push(match variable {
                VarOrConst::Var(variable) => variable.value_at(value_index),
                VarOrConst::Const(constant) => constant.value,
            });
        }
        values.reverse();
        for (variable, value) in self.0.iter().zip(values) {
            ctx.set_value(variable.name().into(), evalexpr::Value::Int(value))
                .map_err(|source| BadVariableError::EvalError { source })?;
        }
        Ok(ctx)
    }
}

fn build_context() -> HashMapContext<DefaultNumericTypes> {
    HashMapContext::new()
    // Add custom variables and functions here
}

custom_error! {pub BadVariableError
    BadName{name: String} = "invalid variable name: '{name}'",
    TooManyValues{name:String, steps:i64}= "the range of values for {name} is too wide: {steps} steps",
    Infinite{name:String}= "the range of values for {name} is incorrect",
    EvalError{source:evalexpr::EvalexprError} = "{source}",
}

#[cfg(test)]
mod tests {
    use evalexpr::Context;

    use super::super::variable::VarOrConst;
    use super::{Variable, Variables};

    #[test]
    fn variable_iteration() {
        let var = Variable {
            name: "hello".to_string(),
            from: 3,
            to: -3,
            step: -3,
        };
        assert_eq!(var.into_iter().collect::<Vec<i64>>(), vec![3, 0, -3]);
    }

    #[test]
    fn variable_validity_check_name() {
        let check = Variable {
            name: "hello world".to_string(),
            from: 0,
            to: 1,
            step: 1,
        }
        .check();
        assert!(
            check
                .unwrap_err()
                .to_string()
                .contains("invalid variable name")
        );
    }

    #[test]
    fn iter_contexts() {
        let vars = Variables(vec![
            VarOrConst::var("x", 0, 1, 1).unwrap(),
            VarOrConst::var("y", 8, 9, 1).unwrap(),
        ]);
        let ctxs: Vec<_> = vars.iter_contexts().collect::<Result<_, _>>().unwrap();
        assert_eq!(4, ctxs.len());
        assert_eq!(Some(&evalexpr::Value::Int(0)), ctxs[0].get_value("x"));
        assert_eq!(Some(&evalexpr::Value::Int(8)), ctxs[0].get_value("y"));

        assert_eq!(Some(&evalexpr::Value::Int(0)), ctxs[1].get_value("x"));
        assert_eq!(Some(&evalexpr::Value::Int(9)), ctxs[1].get_value("y"));

        assert_eq!(Some(&evalexpr::Value::Int(1)), ctxs[2].get_value("x"));
        assert_eq!(Some(&evalexpr::Value::Int(8)), ctxs[2].get_value("y"));

        assert_eq!(Some(&evalexpr::Value::Int(1)), ctxs[3].get_value("x"));
        assert_eq!(Some(&evalexpr::Value::Int(9)), ctxs[3].get_value("y"));
    }

    #[test]
    fn indexed_contexts_match_cartesian_product_order() {
        let vars = Variables(vec![
            VarOrConst::var("x", 0, 1, 1).unwrap(),
            VarOrConst::var("x", 8, 9, 1).unwrap(),
        ]);
        let context = vars.context_at(0).unwrap();
        assert_eq!(Some(&evalexpr::Value::Int(8)), context.get_value("x"));
        assert_eq!(
            Some(&evalexpr::Value::Int(9)),
            vars.context_at(1).unwrap().get_value("x")
        );
    }
}
