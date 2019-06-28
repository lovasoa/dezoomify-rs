use custom_error::custom_error;
use regex::Regex;
use lazy_static::lazy_static;
use evalexpr::HashMapContext;
use itertools::Itertools;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Variable {
    name: String,
    from: i64,
    to: i64,
    step: i64,
}

impl Variable {
    pub fn new(name: &str, from: i64, to: i64, step: i64) -> Result<Variable, BadVariableError> {
        let var = Variable { name: name.to_string(), from, to, step };
        var.check().and(Ok(var))
    }
    fn check(&self) -> Result<(), BadVariableError> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"^\w+$").unwrap();
        }
        if !RE.is_match(&self.name) {
            return Err(BadVariableError::BadName { name: self.name.clone() });
        }
        let steps = (self.to - self.from) / self.step;
        if steps < 0 {
            return Err(BadVariableError::Infinite { name: self.name.clone() });
        } else if steps > std::u32::MAX as i64 {
            return Err(BadVariableError::TooManyValues { name: self.name.clone(), steps });
        }
        Ok(())
    }

    fn in_range(&self, i: i64) -> bool {
        (self.from <= i && i <= self.to) || (self.to <= i && i <= self.from)
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone)]
pub struct VariableIterator<'a> { variable: &'a Variable, current: i64 }

impl<'a> Iterator for VariableIterator<'a> {
    type Item = i64;

    fn next(&mut self) -> Option<Self::Item> {
        if self.variable.in_range(self.current) {
            let current = self.current;
            self.current += self.variable.step;
            Some(current)
        } else {
            None
        }
    }
}

impl<'a> IntoIterator for &'a Variable {
    type Item = i64;
    type IntoIter = VariableIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        VariableIterator { variable: self, current: self.from }
    }
}

#[derive(Deserialize, Debug)]
pub struct Variables(Vec<Variable>);

impl Variables {
    pub fn new(vars: Vec<Variable>) -> Variables {
        Variables(vars)
    }
    pub fn iter_contexts<'a>(&'a self)
                             -> impl Iterator<Item=Result<HashMapContext, BadVariableError>> + 'a {
        self.0.iter().map(|variable| {
            variable.into_iter().map(move |val| (variable.name(), val))
        }).multi_cartesian_product().map(|var_values| {
            // Iterator on all the combination of values for the variables
            use evalexpr::Context;
            let mut ctx = HashMapContext::new();
            for (var_name, var_value) in var_values {
                ctx.set_value(var_name.into(), var_value.into())?;
            }
            Ok(ctx)
        })
    }
}

custom_error! {pub BadVariableError
    BadName{name: String} = "invalid variable name: '{name}'",
    TooManyValues{name:String, steps:i64}= "the range of values for {name} is too wide: {steps} steps",
    Infinite{name:String}= "the range of values for {name} is incorrect",
    EvalError{source:evalexpr::EvalexprError} = "{source}",
}

#[cfg(test)]
mod tests {
    use super::{Variable, BadVariableError, Variables};
    use evalexpr::Context;

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
        let check = Variable { name: "hello world".to_string(), from: 0, to: 1, step: 1 }.check();
        assert!(check.unwrap_err().to_string().contains("invalid variable name"))
    }

    #[test]
    fn iter_contexts() {
        let vars = Variables(vec![
            Variable::new("x", 0, 1, 1).unwrap(),
            Variable::new("y", 8, 9, 1).unwrap(),
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