use crate::value::Value as ExtractorValue;
use crate::workshop::Workshop;

pub fn run<'a>(
    url: &'a str,
    ws: &'a Workshop,
) -> (
    impl ExtractorValue<String> + 'a,
    impl ExtractorValue<String> + 'a,
    impl ExtractorValue<String> + 'a,
    impl ExtractorValue<String> + 'a,
) {
    let webpage = ws.http_get(&url.to_string()).html();
    let inline_metadata = webpage.content_of(r#"selector"#).json();
    let jsonp_url = webpage
        .element(".class")
        .attr("data-foobar")
        .js()
        .index("x")
        .index("y")
        .json_string()
        .relative_to(url.to_string());
    let media_data = ws.http_get(&jsonp_url).jsonp();
    let resource_data = media_data.index("x");
    let metadata = media_data.index("y");

    (
        metadata.index("a").json_string(),
        metadata.index("b").json_string(),
        (
            inline_metadata.index("foo").json_string(),
            webpage.content_of(".other-class"),
        ),
        resource_data
            .indices(&["a", "b"])
            .indices(&["a", "b", "c"])
            .json_string()
            .relative_to(jsonp_url),
    )
}
