extern crate alloc;

use core::fmt::Error;

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::{FromUtf8Error, String};
use alloc::vec::Vec;

use defmt::warn;

pub type CallbackBt = alloc::collections::BTreeMap<&'static str, &'static dyn HttpCallback>;

pub const SUPPORTED_METHODS: [&str; 2] = ["GET", "POST"];

pub enum HttpError {
    ParseError,
    CallbackNotFound,
}

pub enum HttpContentType {
    Text,
    Data,
}

impl HttpContentType {
    fn as_str(&self) -> &'static str {
        match self {
            HttpContentType::Data => "Content-Type: application/data\r\n",
            HttpContentType::Text => "Content-Type: text/html\r\n",
        }
    }
}

pub enum HttpEncodingType {
    None,
    Gzip,
}

impl HttpEncodingType {
    fn as_str(&self) -> &'static str {
        match self {
            HttpEncodingType::Gzip => "Content-Encoding: gzip\r\n",
            _ => "",
        }
    }
}

impl From<FromUtf8Error> for HttpError {
    fn from(_value: FromUtf8Error) -> Self {
        HttpError::ParseError
    }
}





pub fn gen_http_header(
    data: &[u8],
    content_type: HttpContentType,
    encoding_type: Option<HttpEncodingType>,
) -> Vec<u8> {
    let lenstr = format!("Content-Length: {}\r\n", data.len());
    
    let contentstr = content_type.as_str();
    let encodingstr = encoding_type.unwrap_or(HttpEncodingType::None).as_str();
    format!("HTTP/1.1 200 OK\r\n{contentstr}{encodingstr}{lenstr}Connection: close\r\n\r\n").into()
}

pub trait HttpCallback {
    fn handle_request(&self, request: &HttpRequest) ->  Vec<u8>;
}

pub struct Httpserver {
    callbacks: CallbackBt,
}

#[derive(Debug, defmt::Format)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub body: String,
}

pub const HTTP_404_RESPONSE: &[u8] = "HTTP/1.1 404 Not Found\r\n\
                                Content-Type: text/plain\r\n\
                                Content-Length: 13\r\n\
                                Connection: close\r\n\r\n\
                                404 Not Found".as_bytes();

impl Httpserver {
    pub fn new(callbacks: CallbackBt) -> Self {
        Httpserver { callbacks }
    }

    pub fn parse_request(&mut self, request_buf: &[u8]) -> Result< Vec<u8>, HttpError> {
        let req = String::from_utf8(request_buf.to_vec())?;

        // For simplicity, assume that the request is well-formed
        let parts: Vec<&str> = req.lines().collect();

        let method: String = parts[0].split_whitespace().nth(0).unwrap_or("").into();

        if !SUPPORTED_METHODS.iter().any(|x| x == &method.as_str()) {
            return Err(HttpError::ParseError);
        }

        let path = parts[0].split_whitespace().nth(1).unwrap().into();

        let body_index = req.find("\r\n\r\n").unwrap_or(req.len());
        let body = req.get(body_index + 4..).unwrap_or("");

        let request = HttpRequest {
            method,
            path,
            body: body.into(),
        };

        let callback = self
            .callbacks
            .get(request.method.as_str())
            .ok_or(HttpError::CallbackNotFound)?;
        let resp = callback.handle_request(&request);
        Ok(resp) //TODO: this assumes callbacks can never fail!
    }
}
