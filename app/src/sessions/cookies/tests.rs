use super::{CookieRead, read_session};
use axum::http::{HeaderMap, header};

#[test]
fn missing_cookie_is_anonymous() {
    assert!(matches!(
        read_session(&HeaderMap::new()),
        CookieRead::Missing
    ));
}

#[test]
fn invalid_cookie_value_is_invalid() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        "circus_session=not-a-token".parse().unwrap(),
    );
    assert!(matches!(read_session(&headers), CookieRead::Invalid));
}

#[test]
fn duplicate_session_cookies_are_invalid() {
    let mut headers = HeaderMap::new();
    headers.append(
        header::COOKIE,
        "circus_session=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .parse()
            .unwrap(),
    );
    headers.append(
        header::COOKIE,
        "circus_session=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .parse()
            .unwrap(),
    );
    assert!(matches!(read_session(&headers), CookieRead::Invalid));
}
