//! HttpStub: Ad-hoc local servers help you test your HTTP client code.
//!
//! Easily define as many stub servers as you want. Make assertions
//! on the request and build a response to send back.
//!
//! [Fork on GitHub](https://github.com/bitex-la/rust-http-stub)
//!
//! #  Examples
//!
//! ```
//! extern crate http_stub;
//! extern crate hyper;
//!
//! // Your client HTTP code will likely be using hyper too so
//! // this is the recommended way to use http_stub to avoid
//! // name clashing.
//! use self::http_stub as hs;
//! use self::http_stub::HttpStub;
//! 
//! // These modules are for the actual code we're writing and testing.
//! use std::io::Read;
//! use hyper::client::Client;
//! use hyper::status::StatusCode;
//! 
//! fn body_to_string<R: Read>(mut readable: R) -> String{
//!   let ref mut body = vec![];
//!   let _ = readable.read_to_end(body);
//!   String::from_utf8_lossy(body).into_owned()
//! }
//!
//! fn main(){
//!   // Run an HttpStub server. It returns the URL where the server is listening,
//!   // for example: http://127.0.0.1:3001
//!   // It's fixed to listen on 127.0.0.1 and it will use up ports counting up from
//!   // port 3000, each new server will use the next port to make sure there are
//!   // no port conflicts. This does mean you should not be using those ports too.
//!   let server_one: String = HttpStub::run(|mut stub|{
//!     stub.got_body(r"foo=bar");
//!     stub.got_path("/a_post");
//!     stub.got_method(hs::Method::Post);
//!     stub.send_status(hs::StatusCode::NotFound);
//!     stub.send_header(hs::header::ContentType(
//!       hs::Mime(hs::TopLevel::Application, hs::SubLevel::Json, vec![])));
//!
//!     // send_body should always be the last step. It writes the response body and sends it.
//!     // Rendering the 'response' field of the HttpStub unusable.
//!     stub.send_body("number one");
//!   });
//!
//!   let server_two = HttpStub::run(|mut stub|{
//!     // Notice all search strings are actually used for creating a regex.
//!     // That's why we escape the '?' when matching for the path.
//!     stub.got_path(r"/a_get\?foo=bar");
//!     stub.got_method(hs::Method::Get);
//!     stub.send_status(hs::StatusCode::Ok);
//!     stub.send_header(hs::header::ContentType(
//!       hs::Mime(hs::TopLevel::Application, hs::SubLevel::Json, vec![])));
//!     stub.send_body("number two");
//!   });
//!
//!   let client = Client::new();
//!
//!   let response_one = client.post(&format!("{}/a_post", server_one))
//!     .body("foo=bar").send().unwrap();
//!
//!   assert_eq!(response_one.status, StatusCode::NotFound);
//!   assert_eq!(body_to_string(response_one), "number one");
//!
//!   let response_two = client.get(&format!("{}/a_get?foo=bar", server_two))
//!     .send().unwrap();
//!
//!   assert_eq!(response_two.status, StatusCode::Ok);
//!   assert_eq!(body_to_string(response_two), "number two");
//! }
//! ```

pub extern crate hyper;
pub extern crate regex;

pub use self::hyper::method::Method;
pub use self::hyper::status::StatusCode;
pub use hyper::mime::{Mime, TopLevel, SubLevel};
pub use self::hyper::header as header;
pub use self::hyper::mime as mime;
use self::hyper::header::{Header, HeaderFormat};
use hyper::Server as HyperServer;
use hyper::server::Request as HyperRequest;
use hyper::server::Response as HyperResponse;
use hyper::uri::RequestUri as Uri;
use std::thread;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering, ATOMIC_USIZE_INIT};
use regex::Regex;

/// The entry point.
/// Associated functions can create new servers,
/// instances can make assertions and return headers, status and a body.
pub struct HttpStub <'a, 'b: 'a, 'c> {
  pub request: HyperRequest<'a, 'b>,
  pub response: HyperResponse<'c>,
  request_body: String
}

static SERVER_COUNT: AtomicUsize = ATOMIC_USIZE_INIT;

impl <'a, 'b: 'a, 'c> HttpStub<'a, 'b, 'c> {

  /// Start a new server, on a port above 3000.
  /// Returns a string with the server url, for example:
  /// ```text
  /// http://127.0.0.1:3000
  /// ```
  pub fn run<F>(spec: F) -> String
    where F: Fn(HttpStub) + Send + Sync + 'static
  {
    let port = 3000 + SERVER_COUNT.fetch_add(1, Ordering::SeqCst);
    let url = format!("127.0.0.1:{}", port);
    let full_url = format!("http://{}", url);
    thread::spawn(move || {
      HyperServer::http(&*url).unwrap()
        .handle(move |req: HyperRequest, res: HyperResponse| {
          spec(HttpStub::new(req, res));
        }).unwrap();
    });
    full_url
  }

  fn new(mut request: HyperRequest<'a,'b>, response: HyperResponse<'c>) -> HttpStub<'a,'b,'c> {
    let ref mut body = vec![];
    let _ = request.read_to_end(body);
    HttpStub{
      request: request,
      response: response,
      request_body: String::from_utf8_lossy(body).into_owned()
    }
  }

  /// Assert request's method matches the expected Method.
  pub fn got_method(&self, method: Method){
    assert_eq!(self.request.method, method);
  }

  /// Assert request's path matches your expectation.
  pub fn got_path(&self, path: &str){
    match self.request.uri.clone() {
      Uri::AbsolutePath(got) => { HttpStub::assert_match(&got, path) }
      _ => { panic!("Was not absolute path") }
    }
  }

  /// Assert request's header "header_name" matches the given "value".
  /// Value is compiled as a regex so you can provide a pattern.
  pub fn got_header(&self, header_name: &str, value: &str){
    let header = self.request.headers
      .get_raw(header_name).unwrap().first().unwrap();
    let content = String::from_utf8_lossy(header).into_owned();
    HttpStub::assert_match(&content, value);
  }

  /// Assert request's body contains the given value.
  /// Value is compiled as a regex so you can provide a pattern.
  pub fn got_body(&self, value: &str){
    HttpStub::assert_match(&self.request_body, value);
  }

  /// Respond with the given StatusCode
  pub fn send_status(&mut self, status: StatusCode){
    *self.response.status_mut() = status;
  }

  /// Add a header to the response. (Call as many times as you want).
  pub fn send_header<H: Header + HeaderFormat>(&mut self, header: H){
    self.response.headers_mut().set(header);
  }

  /// Sends the response and closes the stream. Pass an empty string
  /// for an empty body. It's mandatory to send a body with your
  /// response and it should be the last thing you do with your HttpStub.
  pub fn send_body(self, body: &str){
    let _ = self.response.send(body.to_string().as_bytes());
  }

  fn assert_match(haystack: &str, needle: &str){
    let re = Regex::new(needle).unwrap();
    assert!(re.is_match(&haystack),
      format!("regex {} did not match text {}", needle, haystack));
  }
}
