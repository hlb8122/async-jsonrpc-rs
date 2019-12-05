// Rust JSON-RPC Library
// Written in 2015 by
//   Andrew Poelstra <apoelstra@wpsoftware.net>
//
// Forked in 2019 by
//   Harry Barber <harrybarber@protonmail.com>
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//

//! # Rust JSON-RPC Library
//!
//! Rust support for the JSON-RPC 2.0 protocol.
//!

#[macro_use]
extern crate serde;

pub mod client;
pub mod error;
mod util;

pub use client::Client;
pub use error::Error;

#[derive(Debug, Clone, PartialEq, Serialize)]
/// Represents the JSONRPC request object.
pub struct Request<'a, 'b> {
    pub method: &'a str,
    pub params: &'b [serde_json::Value],
    pub id: serde_json::Value,
    pub jsonrpc: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
/// Represents the JSONRPC response object.
pub struct Response {
    pub result: Option<serde_json::Value>,
    pub error: Option<error::RpcError>,
    pub id: serde_json::Value,
    pub jsonrpc: Option<String>,
}

impl Response {
    /// Extract the result.
    pub fn result<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        if let Some(e) = &self.error {
            return Err(Error::Rpc(e.clone()));
        }

        T::deserialize(self.result.as_ref().unwrap_or(&serde_json::Value::Null))
            .map_err(Error::Json)
    }

    /// Extract the result, consuming the response.
    pub fn into_result<T: serde::de::DeserializeOwned>(self) -> Result<T, Error> {
        if let Some(e) = self.error {
            return Err(Error::Rpc(e));
        }

        serde_json::from_value(self.result.unwrap_or(serde_json::Value::Null)).map_err(Error::Json)
    }

    /// Returns the [`RpcError`].
    pub fn error(self) -> Option<error::RpcError> {
        self.error
    }

    /// Returns `true` if the result field is [`Some`] value.
    pub fn is_result(&self) -> bool {
        self.result.is_some()
    }

    /// Returns `true` if the error field is [`Some`] value.
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

#[cfg(test)]
mod tests {

    use super::Response;
    use serde_json;

    #[test]
    fn response_is_none() {
        let joanna = Response {
            result: Some(From::from(true)),
            error: None,
            id: From::from(81),
            jsonrpc: Some(String::from("2.0")),
        };

        let bill = Response {
            result: None,
            error: None,
            id: From::from(66),
            jsonrpc: Some(String::from("2.0")),
        };

        assert!(joanna.is_error());
        assert!(bill.is_error());
    }

    #[test]
    fn response_extract() {
        let obj = vec!["Mary", "had", "a", "little", "lamb"];
        let response = Response {
            result: Some(serde_json::to_value(&obj).unwrap()),
            error: None,
            id: serde_json::Value::Null,
            jsonrpc: Some(String::from("2.0")),
        };
        let recovered1: Vec<String> = response.result().unwrap();
        assert!(response.clone().check_error().is_ok());
        let recovered2: Vec<String> = response.into_result().unwrap();
        assert_eq!(obj, recovered1);
        assert_eq!(obj, recovered2);
    }

    #[test]
    fn null_result() {
        let s = r#"{"result":null,"error":null,"id":"test"}"#;
        let response: Response = serde_json::from_str(&s).unwrap();
        let recovered1: Result<(), _> = response.result();
        let recovered2: Result<(), _> = response.clone().into_result();
        assert!(recovered1.is_ok());
        assert!(recovered2.is_ok());

        let recovered1: Result<String, _> = response.result();
        let recovered2: Result<String, _> = response.clone().into_result();
        assert!(recovered1.is_err());
        assert!(recovered2.is_err());
    }

    #[test]
    fn batch_response() {
        // from the jsonrpc.org spec example
        let s = r#"[
            {"jsonrpc": "2.0", "result": 7, "id": "1"},
            {"jsonrpc": "2.0", "result": 19, "id": "2"},
            {"jsonrpc": "2.0", "error": {"code": -32600, "message": "Invalid Request"}, "id": null},
            {"jsonrpc": "2.0", "error": {"code": -32601, "message": "Method not found"}, "id": "5"},
            {"jsonrpc": "2.0", "result": ["hello", 5], "id": "9"}
        ]"#;
        let batch_response: Vec<Response> = serde_json::from_str(&s).unwrap();
        assert_eq!(batch_response.len(), 5);
    }
}
