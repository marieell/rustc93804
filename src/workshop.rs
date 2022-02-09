use crate::http::Client as HttpClient;
use crate::value::*;
use paste::paste;
use std::rc::Rc;

#[derive(Debug)]
pub struct Workshop {
    http_client: Rc<dyn HttpClient + Sync + Send>,
}

impl Workshop {
    pub fn new<H: HttpClient + Send + Sync + 'static>(http_client: H) -> Self {
        Self {
            http_client: Rc::new(http_client),
        }
    }
}

#[derive(Debug)]
pub struct WorkshopValue<'ws, I> {
    inner: Rc<I>,
    ws: &'ws Workshop,
}

impl<'ws, I> WorkshopValue<'ws, I> {
    pub fn new(inner: I, ws: &'ws Workshop) -> Self {
        Self {
            inner: Rc::new(inner),
            ws,
        }
    }
}

impl<V, I> Value<V> for WorkshopValue<'_, I>
where
    I: Value<V>,
{
    type Error = I::Error;
    type Stream = I::Stream;
    fn get(&self) -> Self::Stream {
        self.inner.get()
    }
}

macro_rules! workshop {
  ( @item $fn:ident($( $name:ident: $type:ty ),*): $in:ty => $out:ty, $err:tt ) => {
    pub fn $fn<'ws, V>(&'ws self, input: V, $($name: $type),*) -> WorkshopValue<'ws, impl Value<$out, Error=$err<V::Error>>> where V: Value<$in>, V::Error: Clone {
      // FIXME paste shouldn't be necessary
      WorkshopValue::new(paste! { [<$fn>](input.get(), $($name),*) }, &self)
    }
  };
  ( @wsv_item $fn:ident($( $name:ident: $type:ty ),*): $in:ty => $out:ty, $err:tt ) => {
    pub fn $fn(
      &self,
      $($name: $type),*
    ) -> WorkshopValue<'ws, impl Value<$out, Error=$err<I::Error>>> where I: Value<$in>, I::Error: Clone {
      self.ws.$fn(Rc::clone(&self.inner), $($name),*)
    }
  };
  ( @accum ($fn:ident($( $name:ident: $type:ty ),*): $in:ty => $out:ty, $err:tt;) -> ($($body:tt)*) ($($wsv:tt)*) ) => {
    impl Workshop {
      workshop!{ @item $fn($($name: $type),*): $in => $out, $err }

      $($body)*
    }

    impl<'ws, I> WorkshopValue<'ws, I> {
      workshop!{ @wsv_item $fn($($name: $type),*): $in => $out, $err }

      $($wsv)*
    }
  };
  ( @accum ($fn:ident($( $name:ident: $type:ty ),*): $in:ty => $out:ty, $err:tt; $($tail:tt)*) -> ($($body:tt)*) ($($wsv:tt)*) ) => {
    workshop!{
      @accum ($($tail)*) -> (
        workshop!{ @item $fn($( $name: $type),*): $in => $out, $err }

        $($body)*
      )
      (
        workshop!{ @wsv_item $fn($($name: $type),*): $in => $out, $err }

        $($wsv)*
      )
    }
  };
  ( $($tt:tt)* ) => (
    workshop!{ @accum ($($tt)*) -> ( ) ( ) }
  );
}

impl Workshop {
    pub fn http_get<'ws, V>(
        &'ws self,
        v: &V,
    ) -> WorkshopValue<'ws, impl Value<String, Error = WebpageValueError<V::Error>>>
    where
        V: Value<String>,
        V::Error: Clone,
    {
        WorkshopValue::new(http_get(v.get(), Rc::clone(&self.http_client)), self)
    }

    pub fn html<V>(&'_ self, v: V) -> WorkshopValue<'_, impl Value<scraper::Html, Error = V::Error>>
    where
        V: Value<String>,
        V::Error: Clone,
    {
        WorkshopValue::new(html(v.get()), self)
    }
}

impl<'ws, I> WorkshopValue<'ws, I> {
    pub fn html(&self) -> WorkshopValue<'ws, impl Value<scraper::Html, Error = I::Error>>
    where
        I: Value<String>,
        I::Error: Clone,
    {
        self.ws.html(Rc::clone(&self.inner))
    }
    pub fn relative_to<B>(
        &self,
        base: B,
    ) -> WorkshopValue<'ws, impl Value<String, Error = RelativeToError<I::Error, B::Error>>>
    where
        I: Value<String>,
        I::Error: Clone,
        B: Value<String>,
        B::Error: Clone,
    {
        WorkshopValue::new(relative_to(self.inner.get(), base.get()), self.ws)
    }
}

workshop! {
  element(selector: &'static str): scraper::Html => Element, HtmlElementValueError;
  content_of(selector: &'static str): scraper::Html => String, HtmlContentValueError;
  attr(name: &'static str): Element => String, HtmlAttrValueError;
  js(): String => serde_json::Value, JsValueError;
  json(): String => serde_json::Value, JsonValueError;
  jsonp(): String => serde_json::Value, JsonpValueError;
  index(index: &'static str): serde_json::Value => serde_json::Value, IndexValueError;
  indices(indices: &'static [&'static str]): serde_json::Value => serde_json::Value, IndexValueError;
  json_string(): serde_json::Value => String, JsonStringValueError;
}

#[cfg(test)]
mod test {
    use super::Workshop;

    #[test]
    fn copy_value() {
        let _ws = Workshop::new("");
    }
}
