use crate::http::{Client as HttpClient, ClientError as HttpError};
use futures_util::future;
use futures_util::stream::{self, Stream, StreamExt};
use lazy_regex::regex_captures;
use shared_stream::{Share, Shared};
use std::convert::Infallible;
use std::error::Error as StdError;
use std::fmt::Debug;
use std::ops::Deref;
use thiserror::Error;

pub trait Value<V> {
    type Error: StdError + Clone;
    type Stream: Stream<Item = Rc<Result<V, Self::Error>>>;
    fn get(&self) -> Self::Stream;
}

use std::rc::Rc;
impl<T, V> Value<V> for Rc<T>
where
    T: Value<V>,
{
    type Error = T::Error;
    type Stream = T::Stream;
    fn get(&self) -> Self::Stream {
        let x: &T = &*self;
        x.get()
    }
}

impl Value<String> for String {
    type Error = Infallible;
    type Stream = stream::Once<future::Ready<Rc<Result<String, Self::Error>>>>;
    fn get(&self) -> Self::Stream {
        stream::once(future::ready(Rc::new(Ok(self.clone()))))
    }
}

impl Value<String> for &str {
    type Error = Infallible;
    type Stream = stream::Once<future::Ready<Rc<Result<String, Self::Error>>>>;
    fn get(&self) -> Self::Stream {
        stream::once(future::ready(Rc::new(Ok(self.to_string()))))
    }
}

impl<T: Clone> Value<T> for Vec<T> {
    type Error = Infallible;
    type Stream = impl Stream<Item = Rc<Result<T, Self::Error>>>;
    fn get(&self) -> Self::Stream {
        stream::iter(self.clone().into_iter().map(|v| Rc::new(Ok(v))))
    }
}

#[derive(Error, Clone, Debug, PartialEq)]
pub enum EitherError<A: StdError, B: StdError> {
    #[error(transparent)]
    A(A),
    #[error(transparent)]
    B(#[from] B),
}

impl<T: Clone, A: Value<T>, B: Value<T>> Value<T> for (A, B)
where
    A::Error: StdError + Clone,
    B::Error: StdError + Clone,
{
    type Error = EitherError<A::Error, B::Error>;
    type Stream = impl Stream<Item = Rc<Result<T, Self::Error>>>;
    fn get(&self) -> Self::Stream {
        self.0
            .get()
            .map(|v| Rc::new(v.as_ref().clone().map_err(EitherError::A)))
            .chain(
                self.1
                    .get()
                    .map(|v| Rc::new(v.as_ref().clone().map_err(EitherError::B))),
            )
    }
}

impl<V, E, S> Value<V> for Shared<S>
where
    S: Stream<Item = Rc<Result<V, E>>>,
    E: StdError + Clone,
{
    type Error = E;
    type Stream = Self;
    fn get(&self) -> Self::Stream {
        self.clone()
    }
}

#[derive(Error, Clone, Debug, PartialEq)]
pub enum WebpageValueError<U: StdError> {
    #[error(transparent)]
    Upstream(U),
    #[error("HTTP error: {0}")]
    Http(#[from] HttpError),
}

pub fn http_get<E, I: Stream<Item = Rc<Result<String, E>>>, H: 'static + HttpClient + ?Sized>(
    input: I,
    http_client: Rc<H>,
) -> impl Value<String, Error = WebpageValueError<E>>
where
    E: Clone + StdError,
{
    input
        .then(move |url_result| {
            let http_client = Rc::clone(&http_client);
            async move {
                Rc::new(match url_result.as_ref() {
                    Ok(url) => http_client.get(url).await.map_err(WebpageValueError::Http),
                    Err(e) => Err(WebpageValueError::Upstream((*e).clone())),
                })
            }
        })
        .shared()
}

pub use scraper::node::Element;
use scraper::{Html, Selector};

pub fn html<E: Clone + StdError, I: Stream<Item = Rc<Result<String, E>>>>(
    input: I,
) -> impl Value<Html, Error = E>
where
{
    input
        .map(move |html_result| {
            Rc::new(match html_result.as_ref() {
                Ok(v) => Ok(Html::parse_document(v)),
                Err(e) => Err(e.clone()),
            })
        })
        .shared()
}

#[derive(Error, Clone, Debug, PartialEq)]
pub enum HtmlElementValueError<U: StdError> {
    #[error(transparent)]
    Upstream(U),
    #[error("Could not find element `{0}` in html source `{1}`")]
    ElementMissing(&'static str, String),
}

pub fn element<E: Clone + StdError, I: Stream<Item = Rc<Result<Html, E>>>>(
    input: I,
    selector: &'static str,
) -> impl Value<Element, Error = HtmlElementValueError<E>>
where
{
    input
        .flat_map(move |document_result| {
            stream::iter(
                match document_result.as_ref() {
                    Ok(document) => {
                        // FIXME lazy_static
                        let select = Selector::parse(selector).expect("Could not parse selector");
                        let result: Vec<_> = document
                            .select(&select)
                            .map(|v| Ok(v.value().clone()))
                            .collect();
                        if result.is_empty() {
                            vec![Err(HtmlElementValueError::ElementMissing(
                                selector,
                                document.root_element().html(),
                            ))]
                        } else {
                            result
                        }
                    }
                    Err(e) => vec![Err(HtmlElementValueError::Upstream(e.clone()))],
                }
                .into_iter()
                .map(Rc::new),
            )
        })
        .shared()
}

#[derive(Clone, Error, Debug, PartialEq)]
pub enum HtmlContentValueError<U: StdError> {
    #[error(transparent)]
    Upstream(#[from] U),
    #[error("Could not find element `{0}` in html source `{1}`")]
    ElementMissing(&'static str, String),
}

pub fn content_of<E: Clone + StdError, I: Stream<Item = Rc<Result<Html, E>>>>(
    input: I,
    selector: &'static str,
) -> impl Value<String, Error = HtmlContentValueError<E>>
where
{
    input
        .flat_map(move |document_result| {
            stream::iter(
                match document_result.as_ref() {
                    Ok(document) => {
                        // FIXME lazy_static
                        let select = Selector::parse(selector).expect("Could not parse selector");
                        let result: Vec<_> = document
                            .select(&select)
                            .map(|v| Ok(v.inner_html().trim().to_string()))
                            .collect();
                        if result.is_empty() {
                            vec![Err(HtmlContentValueError::ElementMissing(
                                selector,
                                document.root_element().html(),
                            ))]
                        } else {
                            result
                        }
                    }
                    Err(e) => vec![Err(HtmlContentValueError::Upstream(e.clone()))],
                }
                .into_iter()
                .map(Rc::new),
            )
        })
        .shared()
}

#[derive(Clone, Error, Debug, PartialEq)]
pub enum HtmlAttrValueError<U: StdError> {
    #[error(transparent)]
    Upstream(#[from] U),
    #[error("Element `{0:?}` has no attr `{1}`")]
    AttrMissing(Element, &'static str),
}

pub fn attr<E: Clone + StdError, I: Stream<Item = Rc<Result<Element, E>>>>(
    input: I,
    name: &'static str,
) -> impl Value<String, Error = HtmlAttrValueError<E>> {
    input
        .map(move |element_result| {
            Rc::new(match element_result.as_ref() {
                Ok(element) => element
                    .attr(name)
                    .ok_or_else(|| HtmlAttrValueError::AttrMissing(element.clone(), name))
                    .map(ToString::to_string),
                Err(e) => Err(HtmlAttrValueError::Upstream(e.clone())),
            })
        })
        .shared()
}

#[derive(Clone, Error, Debug)]
pub enum JsonValueError<U: StdError> {
    #[error(transparent)]
    Upstream(#[from] U),
    #[error("Could not parse `{0}` as JSON: {1}")]
    NotJson(String, String),
}

pub fn json<
    S: 'static + Deref<Target = str>,
    E: Clone + StdError,
    I: Stream<Item = Rc<Result<S, E>>>,
>(
    input: I,
) -> impl Value<serde_json::Value, Error = JsonValueError<E>> {
    input
        .map(move |value_result| {
            Rc::new(match value_result.as_ref() {
                Ok(value) => serde_json::from_str(value)
                    .map_err(|e| JsonValueError::NotJson(value.to_string(), e.to_string())),
                Err(e) => Err(JsonValueError::Upstream(e.clone())),
            })
        })
        .shared()
}

#[derive(Clone, Error, Debug)]
pub enum JsValueError<U: StdError> {
    #[error(transparent)]
    Upstream(#[from] U),
    #[error("Could not parse `{0}` as JS value: {1}")]
    NotJs(String, String),
}

pub fn js<E: Clone + StdError, I: Stream<Item = Rc<Result<String, E>>>>(
    input: I,
) -> impl Value<serde_json::Value, Error = JsValueError<E>> {
    input
        .map(move |value_result| {
            Rc::new(match value_result.as_ref() {
                Ok(value) => {
                    // FIXME: This is still bad
                    let value = value.replace('\'', "\"");
                    serde_json::from_str(&value)
                        .map_err(|e| JsValueError::NotJs(value.clone(), e.to_string()))
                }
                Err(e) => Err(JsValueError::Upstream(e.clone())),
            })
        })
        .shared()
}

#[derive(Clone, Error, Debug)]
pub enum JsonpValueError<U: StdError> {
    #[error(transparent)]
    Upstream(#[from] U),
    #[error("Could not parse `{0}` as JSONP value: {1}")]
    NotJsonp(String, String),
}

pub fn jsonp<E: Clone + StdError, I: Stream<Item = Rc<Result<String, E>>>>(
    input: I,
) -> impl Value<serde_json::Value, Error = JsonpValueError<E>> {
    fn parse_it<E: StdError + Clone>(
        input_result: Result<&String, &E>,
    ) -> Result<serde_json::Value, JsonpValueError<E>> {
        let input = input_result.map_err(Clone::clone)?;
        let (_, value) = regex_captures!(r"(?s)^\s*[\w$_.]+\s*\((?P<value>.+)\)\s*;?$", input)
            .ok_or_else(|| {
                JsonpValueError::NotJsonp(input.clone(), "Could not find JSONP wrapping".into())
            })?;
        // FIXME: This is still bad
        let value = value.replace('\'', "\"");
        serde_json::from_str(&value)
            .map_err(|e| JsonpValueError::NotJsonp(value.to_string(), e.to_string()))
    }

    input
        .map(move |value_result| Rc::new(parse_it(value_result.as_ref().as_ref())))
        .shared()
}

#[derive(Clone, Error, Debug)]
pub enum IndexValueError<U: StdError> {
    #[error(transparent)]
    Upstream(#[from] U),
    #[error("Could not find `{0}` in `{1:?}`")]
    MissingKey(String, serde_json::Map<String, serde_json::Value>),
    #[error("`{1:?}` is not {0}")]
    WrongType(&'static str, serde_json::Value),
}

pub fn index<E: Clone + StdError, I: Stream<Item = Rc<Result<serde_json::Value, E>>>>(
    input: I,
    index: &'static str,
) -> impl Value<serde_json::Value, Error = IndexValueError<E>> {
    input
        .map(move |value_result| {
            Rc::new(match value_result.as_ref() {
                Ok(value) => {
                    if let serde_json::Value::Object(v) = value {
                        match v.get(index) {
                            Some(v) => Ok(v.clone()),
                            None => Err(IndexValueError::MissingKey(index.to_string(), v.clone())),
                        }
                    } else {
                        Err(IndexValueError::WrongType("an object", value.clone()))
                    }
                }
                Err(e) => Err(IndexValueError::Upstream(e.clone())),
            })
        })
        .shared()
}

pub fn indices<E: Clone + StdError, I: Stream<Item = Rc<Result<serde_json::Value, E>>>>(
    input: I,
    indices: &'static [&'static str],
) -> impl Value<serde_json::Value, Error = IndexValueError<E>> {
    input
        .flat_map(move |value_result| {
            stream::iter(match value_result.as_ref() {
                Ok(value) => {
                    if let serde_json::Value::Object(v) = value {
                        indices
                            .iter()
                            .map(|&index| {
                                Rc::new(match v.get(index) {
                                    Some(v) => Ok(v.clone()),
                                    None => Err(IndexValueError::MissingKey(
                                        index.to_string(),
                                        v.clone(),
                                    )),
                                })
                            })
                            .collect()
                    } else {
                        vec![Rc::new(Err(IndexValueError::WrongType(
                            "an object",
                            value.clone(),
                        )))]
                    }
                }
                Err(e) => vec![Rc::new(Err(IndexValueError::Upstream(e.clone())))],
            })
        })
        .shared()
}

#[derive(Clone, Error, Debug)]
pub enum JsonStringValueError<U: StdError> {
    #[error(transparent)]
    Upstream(#[from] U),
    #[error("`{1:?}` is not {0}")]
    WrongType(&'static str, serde_json::Value),
}

pub fn json_string<E: Clone + StdError, I: Stream<Item = Rc<Result<serde_json::Value, E>>>>(
    input: I,
) -> impl Value<String, Error = JsonStringValueError<E>> {
    input
        .map(move |value_result| {
            Rc::new(match value_result.as_ref() {
                Ok(value) => match value {
                    serde_json::Value::String(v) => Ok(v.clone()),
                    v => Err(JsonStringValueError::WrongType("a string", v.clone())),
                },
                Err(e) => Err(JsonStringValueError::Upstream(e.clone())),
            })
        })
        .shared()
}

use url::{ParseError as UrlParseError, Url};

#[derive(Clone, Error, Debug)]
pub enum RelativeToError<U: StdError, B: StdError> {
    #[error(transparent)]
    Upstream(#[from] U),
    #[error(transparent)]
    Base(B),
    #[error("Error parsing URL `{0}`: {1}")]
    UrlParseError(String, UrlParseError),
}

pub fn relative_to<
    E: Clone + StdError,
    I: Stream<Item = Rc<Result<String, E>>>,
    EB: Clone + StdError,
    B: Stream<Item = Rc<Result<String, EB>>>,
>(
    input: I,
    base: B,
) -> impl Value<String, Error = RelativeToError<E, EB>> {
    let base = base.shared();
    input
        .flat_map(move |value_result| {
            let base = base.clone();
            match value_result.as_ref() {
                Ok(value) => {
                    let value = value.clone();
                    base.map(move |base_result| {
                        Rc::new(match base_result.as_ref() {
                            Ok(b) => Url::parse(b)
                                .and_then(|b| b.join(&value))
                                .map(|v| v.to_string())
                                .map_err(|e| RelativeToError::UrlParseError(b.clone(), e)),
                            Err(e) => Err(RelativeToError::Base(e.clone())),
                        })
                    })
                    .left_stream()
                }
                Err(e) => stream::once(future::ready(Rc::new(Err(RelativeToError::Upstream(
                    e.clone(),
                )))))
                .right_stream(),
            }
        })
        .shared()
}
