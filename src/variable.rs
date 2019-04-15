use custom_error::custom_error;
use regex::Regex;
use lazy_static::lazy_static;

#[derive(Clone, Debug)]
pub struct Variable {
    name: String,
    from: i64,
    to: i64,
    step: i64,
}

impl Variable {
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
}

pub struct VariableIterator<'a> { variable: &'a Variable, current: i64 }

impl<'a> Iterator for VariableIterator<'a> {
    type Item = i64;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.current + self.variable.step;
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

custom_error! {pub BadVariableError
    BadName{name: String} = "invalid name: '{name}'",
    TooManyValues{name:String, steps:i64}= "the range of values for {name} is too wide: {steps} steps",
    Infinite{name:String}= "the range of values for {name} is incorrect",
}

#[cfg(test)]
mod tests {
    use super::{Variable, BadVariableError};

    #[test]
    fn variable_iteration() {
        let var = Variable {
            name: "hello".to_string(),
            from: 3,
            to: -2,
            step: -1,
        };
        assert_eq!(var.into_iter().collect::<Vec<i64>>(), vec![3, 2, 1, 0, -1, -2]);
    }

    #[test]
    fn variable_validity_check() {
        let check = Variable { name: "hello world".to_string(), from: 0, to: 1, step: 1 }.check();
        assert!(check.is_err())
    }
}