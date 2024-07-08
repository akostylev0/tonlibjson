use itertools::Itertools;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockCriteria {
    Seqno { shard: i64, seqno: i32 },
    LogicalTime(i64),
}

#[derive(Debug, Clone, Copy)]
pub enum Route {
    Block { chain: i32, criteria: BlockCriteria },
    Latest,
}

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("route is not available at this moment")]
    RouteNotAvailable,
    #[error("route is unknown")]
    RouteUnknown,
}

pub trait Routed {
    fn contains(&self, chain: &i32, criteria: &BlockCriteria) -> bool;
    fn contains_not_available(&self, chain: &i32, criteria: &BlockCriteria) -> bool;
    fn last_seqno(&self) -> Option<i32>;
}

impl Route {
    pub fn choose<'a, S: Routed, I: IntoIterator<Item=&'a S>>(&self, from: I) -> Result<Vec<&'a S>, RouterError> {
        match self {
            Route::Block { chain, criteria } => {
                let mut known = false;
                let clients: Vec<&S> = from
                    .into_iter()
                    .filter(|s| {
                        if s.contains(chain, criteria) {
                            true
                        } else {
                            if s.contains_not_available(chain, criteria) {
                                known = true;
                            }

                            false
                        }
                    })
                    .collect();

                if clients.is_empty() {
                    if known {
                        Err(RouterError::RouteNotAvailable)
                    } else {
                        Err(RouterError::RouteUnknown)
                    }
                } else {
                    Ok(clients)
                }
            }
            Route::Latest => {
                let groups = from
                    .into_iter()
                    .filter_map(|s| s.last_seqno().map(|seqno| (s, seqno)))
                    .sorted_unstable_by_key(|(_, seqno)| -seqno)
                    .chunk_by(|(_, seqno)| *seqno);

                if let Some((_, group)) = groups.into_iter().next() {
                    return Ok(group
                        .into_iter()
                        .map(|(s, _)| s)
                        .collect());
                }

                Err(RouterError::RouteUnknown)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MyRouted {
        contains: bool,
        contains_not_available: bool,
        last_seqno: Option<i32>,
    }

    impl Routed for MyRouted {
        fn contains(&self, _: &i32, _: &BlockCriteria) -> bool { self.contains }
        fn contains_not_available(&self, _: &i32, _: &BlockCriteria) -> bool { self.contains_not_available }
        fn last_seqno(&self) -> Option<i32> { self.last_seqno }
    }

    #[test]
    fn given_routed_is_empty() {
        let route = Route::Latest;
        let from: Vec<MyRouted> = Vec::new();

        let result = route.choose(&from).unwrap_err();

        assert!(matches!(result, RouterError::RouteUnknown));
    }

    #[test]
    fn given_block_available() {
        let route = Route::Block { chain: 1, criteria: BlockCriteria::LogicalTime(100) };
        let routed = MyRouted {
            contains: true,
            contains_not_available: true,
            last_seqno: None,
        };
        let from = vec![routed.clone()];

        let result = route.choose(&from).unwrap();

        assert_eq!(result, vec![&routed]);
    }

    #[test]
    fn given_block_unknown() {
        let route = Route::Block { chain: 1, criteria: BlockCriteria::LogicalTime(100) };
        let from = vec![MyRouted {
            contains: false,
            contains_not_available: false,
            last_seqno: None,
        }];

        let result = route.choose(&from).unwrap_err();

        assert!(matches!(result, RouterError::RouteUnknown));
    }

    #[test]
    fn given_block_not_available() {
        let route = Route::Block { chain: 1, criteria: BlockCriteria::LogicalTime(100) };
        let from = vec![MyRouted {
            contains: false,
            contains_not_available: true,
            last_seqno: None,
        }, MyRouted {
            contains: false,
            contains_not_available: false,
            last_seqno: None,
        }];

        let result = route.choose(&from).unwrap_err();

        assert!(matches!(result, RouterError::RouteNotAvailable));
    }

    #[test]
    fn route_latest_to_max_seqno() {
        let route = Route::Latest;
        let from = vec![MyRouted {
            contains: false,
            contains_not_available: true,
            last_seqno: Some(70),
        }, MyRouted {
            contains: false,
            contains_not_available: true,
            last_seqno: Some(100),
        }, MyRouted {
            contains: false,
            contains_not_available: true,
            last_seqno: Some(50),
        }];

        let result = route.choose(&from).unwrap();

        assert_eq!(result, vec![&MyRouted {
            contains: false,
            contains_not_available: true,
            last_seqno: Some(100),
        }]);
    }
}
