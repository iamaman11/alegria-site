use dashmap::DashMap;

#[derive(Default)]
pub struct RuntimeStore {
    pub values: DashMap<String, Vec<u8>>,
}
