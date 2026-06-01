use axum::{body::Bytes, http::HeaderMap, response::Response};
use reqwest::Client;

use crate::{
    ai_gateway::provider::LlmProvider,
    app::errors::GatewayError,
    models::registry::ModelRegistry,
    providers::{
        anthropic::{
            client::send_messages,
            stream::response_from_upstream,
            transformation::{parse_body, requested_model, AnthropicMessagesTransformation},
        },
        base::MessagesTransformation,
    },
};

pub type MessagesResponse = Response;

pub struct MessagesCreateRequest {
    pub headers: HeaderMap,
    pub body: Bytes,
}

pub struct MessagesGateway<'a> {
    http: &'a Client,
    models: &'a ModelRegistry,
}

impl<'a> MessagesGateway<'a> {
    pub fn new(http: &'a Client, models: &'a ModelRegistry) -> Self {
        Self { http, models }
    }

    pub async fn create(
        &self,
        request: MessagesCreateRequest,
    ) -> Result<MessagesResponse, GatewayError> {
        let body = parse_body(&request.body)?;
        let deployment = self.models.resolve(requested_model(&body)?)?;

        match deployment.provider {
            LlmProvider::Anthropic => {
                let transform = AnthropicMessagesTransformation;
                let prepared = transform.transform_request(body, deployment)?;
                let stream = prepared.stream;
                let upstream =
                    send_messages(self.http, &request.headers, deployment, prepared).await?;
                let response_headers =
                    transform.transform_response_headers(upstream.headers(), stream);

                Ok(response_from_upstream(upstream, response_headers).await)
            }
        }
    }
}
