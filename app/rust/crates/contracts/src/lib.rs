pub mod generated {
    pub mod alegria {
        pub mod condition {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/alegria.condition.v1.rs"));
            }
        }
        pub mod read_api {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/alegria.read_api.v1.rs"));
            }
        }
        pub mod rules {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/alegria.rules.v1.rs"));
            }
        }
        pub mod sync {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/alegria.sync.v1.rs"));
            }
        }
        pub mod temporal {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/alegria.temporal.v1.rs"));
            }
        }
    }
}

pub mod wire {
    pub mod condition {
        pub use crate::generated::alegria::condition::v1::*;
    }

    pub mod sync {
        pub use crate::generated::alegria::sync::v1::*;
    }

    pub mod rules {
        pub use crate::generated::alegria::rules::v1::*;
    }

    pub mod temporal {
        pub use crate::generated::alegria::temporal::v1::*;
    }
}
