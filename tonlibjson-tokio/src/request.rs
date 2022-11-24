use std::time::Duration;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use serde_json::Value;

pub type RequestId = Uuid;

#[derive(Serialize, Clone)]
pub struct Request {
    #[serde(rename="@extra")]
    pub id: RequestId,

    #[serde(skip_serializing)]
    pub timeout: Duration,

    #[serde(flatten)]
    pub data: Value
}

#[derive(Deserialize, Debug)]
pub struct Response {
    #[serde(rename="@extra")]
    pub id: RequestId,

    #[serde(flatten)]
    pub data: Value
}

impl Request {
    pub fn new<T: Serialize>(data: T) -> anyhow::Result<Self> {
        Ok(Self::with_timeout(data, Duration::from_secs(3))?)
    }

    pub fn with_timeout<T: Serialize>(data: T, timeout: Duration) -> anyhow::Result<Self> {
        Ok(Self {
            id: RequestId::new_v4(),
            timeout,
            data: serde_json::to_value(data)?
        })
    }

    pub fn with_new_id(&self) -> Self {
        Self {
            id: RequestId::new_v4(),
            timeout: self.timeout,
            data: self.data.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::time::Duration;
    use serde_json::json;
    use uuid::Uuid;
    use crate::request::Request;

    #[test]
    fn data_is_flatten() {
        let request = Request {
            id: Uuid::from_str("7431f198-7514-40ff-876c-3e8ee0a311ba").unwrap(),
            timeout: Duration::from_secs(3),
            data: json!({
                "data": "is flatten"
            })
        };

        assert_eq!(serde_json::to_string(&request).unwrap(), "{\"@extra\":\"7431f198-7514-40ff-876c-3e8ee0a311ba\",\"data\":\"is flatten\"}")
    }
}
