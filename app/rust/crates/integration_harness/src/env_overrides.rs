use std::collections::BTreeMap;

#[derive(Debug)]
pub struct EnvOverrideGuard {
    previous: Vec<(String, Option<String>)>,
}

impl EnvOverrideGuard {
    pub fn apply(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut previous = Vec::new();
        for (key, value) in vars {
            previous.push((key.clone(), std::env::var(&key).ok()));
            unsafe {
                std::env::set_var(&key, value);
            }
        }
        Self { previous }
    }
}

impl Drop for EnvOverrideGuard {
    fn drop(&mut self) {
        for (key, previous) in self.previous.drain(..).rev() {
            match previous {
                Some(value) => unsafe {
                    std::env::set_var(&key, value);
                },
                None => unsafe {
                    std::env::remove_var(&key);
                },
            }
        }
    }
}

pub fn btree_env(vars: impl IntoIterator<Item = (String, String)>) -> BTreeMap<String, String> {
    vars.into_iter().collect()
}
