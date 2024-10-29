use crate::deserializer::{Deserialize, DeserializeBoxed, Deserializer, DeserializerBoxedError};
use crate::serializer::{Serialize, Serializer};

pub trait Functional {
    type Result;
}

// TODO[akostylev0] review
pub type Double = f64;
pub type Int31 = i32; // "#" / nat type
pub type Int32 = i32;
pub type Int = i32;
pub type Int53 = i64;
pub type Int64 = i64;
pub type Long = i64;
pub type Int128 = i128;
pub type Int256 = [u8; 32];
pub type Bytes = Vec<u8>;
pub type Object = Bytes;
pub type SecureString = String;
pub type SecureBytes = Vec<u8>;
pub type Vector<T> = Vec<T>;

impl Serialize for Vector<Int256> {
    fn serialize(&self, se: &mut Serializer) {
        se.reserve(4 + 32 * self.len());
        se.write_i31(self.len() as i32);
        for val in self {
            se.write_i256(val)
        }
    }
}

impl Deserialize for Vector<Int256> {
    fn deserialize(de: &mut Deserializer) -> Result<Self, DeserializerBoxedError> {
        let len = de.parse_i31()?;
        let mut buf = Vec::with_capacity(len as usize);
        for _ in 0..len {
            let val = de.parse_i256()?;
            buf.push(val)
        }

        Ok(buf)
    }
}

impl Serialize for Vector<Int32> {
    fn serialize(&self, se: &mut Serializer) {
        se.reserve(4 + 4 * self.len());
        se.write_i31(self.len() as i32);
        for val in self {
            se.write_i32(*val)
        }
    }
}

impl Deserialize for Vector<Int32> {
    fn deserialize(de: &mut Deserializer) -> Result<Self, DeserializerBoxedError> {
        let len = de.parse_i31()?;
        let mut buf = Vec::with_capacity(len as usize);
        for _ in 0..len {
            let val = de.parse_i32()?;
            buf.push(val)
        }

        Ok(buf)
    }
}

impl Serialize for Vector<Int64> {
    fn serialize(&self, se: &mut Serializer) {
        se.reserve(4 + 8 * self.len());
        se.write_i31(self.len() as i32);
        for val in self {
            se.write_i64(*val)
        }
    }
}

impl Deserialize for Vector<Int64> {
    fn deserialize(de: &mut Deserializer) -> Result<Self, DeserializerBoxedError> {
        let len = de.parse_i31()?;
        let mut buf = Vec::with_capacity(len as usize);
        for _ in 0..len {
            let val = de.parse_i64()?;
            buf.push(val)
        }

        Ok(buf)
    }
}

impl<T> Serialize for Vector<T>
where
    T: Serialize,
{
    fn serialize(&self, se: &mut Serializer) {
        se.write_i31(self.len() as i32);
        for val in self {
            val.serialize(se)
        }
    }
}

impl<T> Deserialize for Vector<T>
where
    T: Deserialize,
{
    fn deserialize(de: &mut Deserializer) -> Result<Self, DeserializerBoxedError> {
        let len = de.parse_i31()?;
        let mut buf = Vec::with_capacity(len as usize);
        for _ in 0..len {
            let val = T::deserialize(de)?;
            buf.push(val)
        }

        Ok(buf)
    }
}

impl<T, E> Deserialize for Result<T, E>
where
    T: DeserializeBoxed,
    E: DeserializeBoxed,
{
    fn deserialize(de: &mut Deserializer) -> Result<Self, DeserializerBoxedError> {
        let constructor_number = de.parse_constructor_numer()?;

        Self::deserialize_boxed(constructor_number, de)
    }
}

impl<T, E> DeserializeBoxed for Result<T, E>
where
    T: DeserializeBoxed,
    E: DeserializeBoxed,
{
    fn deserialize_boxed(
        constructor_number: u32,
        de: &mut Deserializer,
    ) -> Result<Self, DeserializerBoxedError> {
        match T::deserialize_boxed(constructor_number, de) {
            Ok(val) => Ok(Ok(val)),
            Err(DeserializerBoxedError::UnexpectedConstructorNumber { .. }) => {
                Ok(Err(E::deserialize_boxed(constructor_number, de)?))
            }
            Err(DeserializerBoxedError::DeserializeError(e)) => {
                Err(DeserializerBoxedError::DeserializeError(e))
            }
        }
    }
}
