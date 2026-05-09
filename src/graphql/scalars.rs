use async_graphql::{InputValueError, InputValueResult, Scalar, ScalarType, Value};

use crate::app::model::MagnetUri as DomainMagnetUri;

/// GraphQL scalar for a magnet URI.
/// Accepts any string that passes `MagnetUri`'s domain validation (valid URL, `magnet:` scheme).
pub struct MagnetUri(pub DomainMagnetUri);

#[Scalar(name = "MagnetUri")]
impl ScalarType for MagnetUri {
    fn parse(value: Value) -> InputValueResult<Self> {
        match value {
            Value::String(s) => s
                .parse::<DomainMagnetUri>()
                .map(MagnetUri)
                .map_err(|e| InputValueError::custom(e.to_string())),
            _ => Err(InputValueError::expected_type(value)),
        }
    }

    fn to_value(&self) -> Value {
        Value::String(self.0.as_str().to_owned())
    }
}
