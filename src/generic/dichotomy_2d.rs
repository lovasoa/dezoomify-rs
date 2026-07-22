#[derive(Default, Debug)]
pub struct Dichotomy {
    min: u32,
    max: Option<u32>,
}

impl Dichotomy {
    fn best_guess(&self) -> u32 {
        if let Some(max) = self.max {
            u32::midpoint(max, self.min)
        } else {
            self.min * 3 + 1
        }
    }
    fn next(&mut self, previous_success: bool) -> Option<u32> {
        let last_guess = self.best_guess();
        if previous_success {
            self.min = last_guess;
        } else {
            self.max = Some(last_guess);
        }
        let next_guess = self.best_guess();
        if next_guess == last_guess {
            None
        } else {
            Some(next_guess)
        }
    }
}

#[derive(Debug)]
pub enum Dichotomy2d {
    Diagonal(Dichotomy),
    Orientation {
        diagonal: u32,
    },
    LastDim {
        diagonal: u32,
        is_landscape: bool,
        last_dim: Dichotomy,
    },
}

impl Dichotomy2d {
    pub fn next(&mut self, previous_success: bool) -> Option<(u32, u32)> {
        let mut next = None;
        let res = match self {
            Dichotomy2d::Diagonal(d) => {
                if let Some(n) = d.next(previous_success) {
                    Some((n, n))
                } else {
                    let diagonal = d.best_guess();
                    next = Some(Dichotomy2d::Orientation { diagonal });
                    Some((diagonal + 1, diagonal))
                }
            }
            Dichotomy2d::Orientation { diagonal } => {
                let dichotomy = Dichotomy {
                    min: *diagonal + u32::from(previous_success),
                    max: None,
                };
                let best = dichotomy.best_guess();
                next = Some(Dichotomy2d::LastDim {
                    diagonal: *diagonal,
                    is_landscape: previous_success,
                    last_dim: dichotomy,
                });
                if previous_success {
                    Some((best, *diagonal))
                } else {
                    Some((*diagonal, best))
                }
            }
            Dichotomy2d::LastDim {
                diagonal,
                is_landscape,
                last_dim,
            } => last_dim.next(previous_success).map(|next| {
                if *is_landscape {
                    (next, *diagonal)
                } else {
                    (*diagonal, next)
                }
            }),
        };
        if let Some(next) = next {
            *self = next;
        }
        res
    }
}

impl Default for Dichotomy2d {
    fn default() -> Self {
        Dichotomy2d::Diagonal(Dichotomy::default())
    }
}

#[test]
fn test_dichotomy1d() {
    for mystery in 0..1000 {
        let mut d = Dichotomy::default();
        let mut tries = 1;
        while let Some(prop) = d.next(d.best_guess() <= mystery) {
            tries += 1;
            assert!(tries <= 20, "guessed {prop} on {tries}th try");
        }
        assert_eq!(
            d.best_guess(),
            mystery,
            "Guessed {} instead of {} in {} tries",
            d.best_guess(),
            mystery,
            tries
        );
    }
}

#[test]
fn test_dichotomy2d() {
    for x in 0..10 {
        for y in 0..10 {
            let mut d = Dichotomy2d::default();
            let mut tries = 1;
            let mut guess = (1, 1);
            while let Some(g) = d.next(guess.0 <= x && guess.1 <= y) {
                guess = g;
                tries += 1;
                assert!(tries <= 20, "guessed {g:?} on {tries}th try");
            }
            assert_eq!(
                guess,
                (x, y),
                "Guessed {:?} instead of {:?} in {} tries",
                guess,
                (x, y),
                tries
            );
        }
    }
}
