use evalexpr::{ContextWithMutableVariables, HashMapContext};
use itertools::Itertools;
use regex::Regex;
use serde::Deserialize;

use custom_error::custom_error;
use lazy_static::lazy_static;

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
        lazy_static! {
            static ref RE: Regex = Regex::new(r"^\w+$").unwrap();
        }
        if !RE.is_match(&self.name) {
            return Err(BadVariableError::BadName {
                name: self.name.clone(),
            });
        }
        let steps = (self.to - self.from) / self.step;
        if steps < 0 {
            return Err(BadVariableError::Infinite {
                name: self.name.clone(),
            });
        } else if steps > i64::from(u32::MAX) {
            return Err(BadVariableError::TooManyValues {
                name: self.name.clone(),
                steps,
            });
        }
        Ok(())
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

impl<'a> IntoIterator for &'a Variable {
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

impl<'a> IntoIterator for &'a VarOrConst {
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

#[derive(Deserialize, Debug)]
pub struct Variables(Vec<VarOrConst>);

impl Variables {
    #[cfg(test)]
    pub fn new(vars: Vec<VarOrConst>) -> Variables {
        Variables(vars)
    }
    pub fn iter_contexts(
        &self,
    ) -> impl Iterator<Item = Result<HashMapContext, BadVariableError>> + '_ {
        self.0
            .iter()
            .map(|variable| variable.into_iter().map(move |val| (variable.name(), val)))
            .multi_cartesian_product()
            .map(|var_values| {
                // Iterator on all the combination of values for the variables
                let mut ctx = build_context();
                for (var_name, var_value) in var_values {
                    ctx.set_value(var_name.into(), var_value.into())?;
                }
                Ok(ctx)
            })
    }
}

fn build_context() -> HashMapContext {
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
        assert!(check
            .unwrap_err()
            .to_string()
            .contains("invalid variable name"))
    }

    #[test]
    fn iter_contexts() {
        let vars = Variables(vec![
            VarOrConst::var("x", 0, 1, 1).unwrap(),
            VarOrConst::var("y", 8, 9, 1).unwrap(),
        ]);
        let ctxs: Vec<_> = vars.iter_contexts().collect::<Result<_, _>>().unwrap();
        assert_eq!(4, ctxs.len());
        assert_eq!(Some(&0.into()), ctxs[0].get_value("x"));
        assert_eq!(Some(&8.into()), ctxs[0].get_value("y"));

        assert_eq!(Some(&0.into()), ctxs[1].get_value("x"));
        assert_eq!(Some(&9.into()), ctxs[1].get_value("y"));

        assert_eq!(Some(&1.into()), ctxs[2].get_value("x"));
        assert_eq!(Some(&8.into()), ctxs[2].get_value("y"));

        assert_eq!(Some(&1.into()), ctxs[3].get_value("x"));
        assert_eq!(Some(&9.into()), ctxs[3].get_value("y"));
    }
}
