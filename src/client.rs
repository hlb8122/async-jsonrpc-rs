// Rust JSON-RPC Library
// Written in 2015 by
//     Andrew Poelstra <apoelstra@wpsoftware.net>
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

//! # Client support
//!
//! Support for connecting to JSONRPC servers over HTTP, sending requests,
//! and parsing responses
//!

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::io::Read;

use hyper::{
    client::{Client as HyperClient, HttpConnector, connect::Connect},
    header::{AUTHORIZATION, CONTENT_TYPE},
    Body,
};
use hyper_tls::{Error as TlsError, HttpsConnector};
use futures_util::TryStreamExt;

use crate::{error::Error, util::HashableValue, Request, Response};

/// A handle to a remote JSONRPC server
pub struct Client<C> {
    url: String,
    user: Option<String>,
    pass: Option<String>,
    client: HyperClient<C, Body>,
    nonce: Arc<Mutex<u64>>,
}

impl<C> Client<C>
where
    C: Connect + Sync + 'static,
    C::Transport: 'static,
    C::Future: 'static,
{
    /// Creates a new client
    pub fn new(url: String, user: Option<String>, pass: Option<String>) -> Client<HttpConnector> {
        // Check that if we have a password, we have a username; other way around is ok
        debug_assert!(pass.is_none() || user.is_some());

        Client {
            url: url,
            user: user,
            pass: pass,
            client: HyperClient::new(),
            nonce: Arc::new(Mutex::new(0)),
        }
    }

    /// Creates a new TLS client
    pub fn new_tls(
        url: String,
        user: Option<String>,
        pass: Option<String>,
    ) -> Result<Client<HttpsConnector<HttpConnector>>, TlsError> {
        // Check that if we have a password, we have a username; other way around is ok
        debug_assert!(pass.is_none() || user.is_some());
        let https = HttpsConnector::new()?;
        let https_client = HyperClient::builder().build::<_, Body>(https);
        Ok(Client {
            url: url,
            user: user,
            pass: pass,
            client: https_client,
            nonce: Arc::new(Mutex::new(0)),
        })
    }

    /// Make a request and deserialize the response
    pub async fn do_rpc<T: for<'a> serde::de::Deserialize<'a>>(
        &self,
        rpc_name: &str,
        args: &[serde_json::value::Value],
    ) -> Result<T, Error> {
        let request = self.build_request(rpc_name, args);
        let response = self.send_request(&request).await?;

        Ok(response.into_result()?)
    }

    /// The actual send logic used by both [send_request] and [send_batch].
    async fn send_raw<B, R>(&self, body_raw: &B) -> Result<R, Error>
    where
        B: serde::ser::Serialize,
        R: for<'de> serde::de::Deserialize<'de>,
    {
        let json_raw = serde_json::to_vec(body_raw).unwrap(); // This is safe
        let body = Body::from(json_raw);
        let mut builder = hyper::Request::post(&self.url);

        // Add authorization
        if let Some(ref user) = self.user {
            let pass_str = match &self.pass {
                Some(some) => some,
                None => "",
            };
            builder = builder.header(AUTHORIZATION, format!("Basic {}:{}", user, pass_str))
        };
        let request = builder.body(body).unwrap(); // This is safe

        // Send request
        let response = self.client.request(request).await?;
        let body = response.into_body().try_concat().await?;
        let parsed: R = serde_json::from_slice(&body)?;

        Ok(parsed)
    }

    /// Sends a request to a client
    pub async fn send_request(&self, request: &Request<'_, '_>) -> Result<Response, Error> {
        let response: Response = self.send_raw(&request).await?;
        if response.jsonrpc != None && response.jsonrpc != Some(From::from("2.0")) {
            return Err(Error::VersionMismatch);
        }
        if response.id != request.id {
            return Err(Error::NonceMismatch);
        }
        Ok(response)
    }

    /// Sends a batch of requests to the client.  The return vector holds the response
    /// for the request at the corresponding index.  If no response was provided, it's [None].
    ///
    /// Note that the requests need to have valid IDs, so it is advised to create the requests
    /// with [build_request].
    pub async fn send_batch(&self, requests: &[Request<'_, '_>]) -> Result<Vec<Option<Response>>, Error> {
        if requests.len() < 1 {
            return Err(Error::EmptyBatch);
        }

        // If the request body is invalid JSON, the response is a single response object.
        // We ignore this case since we are confident we are producing valid JSON.
        let responses: Vec<Response> = self.send_raw(&requests).await?;
        if responses.len() > requests.len() {
            return Err(Error::WrongBatchResponseSize);
        }

        // To prevent having to clone responses, we first copy all the IDs so we can reference
        // them easily. IDs can only be of JSON type String or Number (or Null), so cloning
        // should be inexpensive and require no allocations as Numbers are more common.
        let ids: Vec<serde_json::Value> = responses.iter().map(|r| r.id.clone()).collect();
        // First index responses by ID and catch duplicate IDs.
        let mut resp_by_id = HashMap::new();
        for (id, resp) in ids.iter().zip(responses.into_iter()) {
            if let Some(dup) = resp_by_id.insert(HashableValue(&id), resp) {
                return Err(Error::BatchDuplicateResponseId(dup.id));
            }
        }
        // Match responses to the requests.
        let results =
            requests.into_iter().map(|r| resp_by_id.remove(&HashableValue(&r.id))).collect();

        // Since we're also just producing the first duplicate ID, we can also just produce the
        // first incorrect ID in case there are multiple.
        if let Some(incorrect) = resp_by_id.into_iter().nth(0) {
            return Err(Error::WrongBatchResponseId(incorrect.1.id));
        }

        Ok(results)
    }

    /// Builds a request
    pub fn build_request<'a, 'b>(
        &self,
        name: &'a str,
        params: &'b [serde_json::Value],
    ) -> Request<'a, 'b> {
        let mut nonce = self.nonce.lock().unwrap();
        *nonce += 1;
        Request {
            method: name,
            params: params,
            id: From::from(*nonce),
            jsonrpc: Some("2.0"),
        }
    }

    /// Accessor for the last-used nonce
    pub fn last_nonce(&self) -> u64 {
        *self.nonce.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanity() {
        let client = Client::new("localhost".to_owned(), None, None);
        assert_eq!(client.last_nonce(), 0);
        let req1 = client.build_request("test", &[]);
        assert_eq!(client.last_nonce(), 1);
        let req2 = client.build_request("test", &[]);
        assert_eq!(client.last_nonce(), 2);
        assert!(req1 != req2);
    }
}
