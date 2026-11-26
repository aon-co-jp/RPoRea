//! 逆プロキシ試験用: RPoRea の `start_route` が解決した接続元を返す。
//! 使い方: `RPOREA_TRUSTED_PROXIES=127.0.0.1 whoami 8770`
use rporea::poem::{get, handler_fn, Route, StatusCode};
use rporea::poem::hyper_compat::Response;

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(8770);
    let h = handler_fn(|req, _| async move {
            let h = |k: &str| req.headers().get(k).and_then(|v| v.to_str().ok()).unwrap_or("-").to_string();
            let leaked = req.headers().keys().any(|k| k.as_str().starts_with("x-forwarded-"));
            let body = format!(
                "path={} client={} proto={} host={} prefix={} xff_leaked={}\n",
                req.uri().path(), h("x-rporea-client-ip"), h("x-rporea-proto"), h("host"), h("x-rporea-prefix"), leaked
            );
            let mut r = Response::new(rporea::poem::fixed_body(body.into()));
            *r.status_mut() = StatusCode::OK;
            r
        });
    let route = Route::new().at("/", get(h.clone())).at("/x/y", get(h));
    let trusted = rporea::behind_proxy::trusted_from_env().expect("RPOREA_TRUSTED_PROXIES");
    let (_a, h) = rporea::start_route(route, trusted, ([127, 0, 0, 1], port)).await.expect("bind");
    let _ = h.await;
}
