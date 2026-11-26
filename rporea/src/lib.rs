//! RPoRea: RPoem(サーバー)と Reaxt(Rust製フルスタック枠組み)の融合の入口。
//!
//! - `reaxt` / `reaxt_rpoem` / `open_runo_poem_compat` を1か所から再公開する。
//! - `start` / `serve`: Reaxt の `App` を RPoem 上で(トレース+圧縮つきの本番向け既定で)起動する。
//!   RPoem の `Server::run` は「起動して返す」形で、返る `JoinHandle` を待たないとプロセスが終わるため、
//!   `serve` はそれを待って起動し続ける(`start` はアドレスとハンドルを返す=試験・停止制御向け)。
//!
//! 3姉妹(Reaxt・RPoRea・open-cosmo)は常に同時開発する(`docs/THREE-SISTERS.md`)。
pub use open_runo_poem_compat as poem;
pub mod behind_proxy;
pub use reaxt;
pub use reaxt_rpoem;

use open_runo_poem_compat::{Server, TcpListener};
use reaxt::app::App;
use std::net::SocketAddr;
use std::sync::Arc;

/// `app` を RPoem 上で起動し、実際に bind したアドレスとサーバータスクを返す。
///
/// 逆プロキシ(nginx・Apache)の背後: 環境変数 `RPOREA_TRUSTED_PROXIES`(例 `127.0.0.1, 10.0.0.0/8`)に
/// 信頼するプロキシを書くと、その接続元からの `X-Forwarded-*` だけを信じて `x-rporea-client-ip` などに直す。
/// 未設定でも、偽の `X-Forwarded-*` は必ず取り除かれる(`behind_proxy::apply`)。値が不正なら起動しない。
pub async fn start(app: Arc<App>, addr: impl Into<SocketAddr>) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let trusted = behind_proxy::trusted_from_env()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{}: {e}", behind_proxy::TRUSTED_ENV)))?;
    start_route(reaxt_rpoem::route_production(app), trusted, addr).await
}

/// 任意の `Route` を、信頼プロキシの一覧つきで起動する(`start` の中身。試験や独自ルート向け)。
pub async fn start_route(
    route: poem::Route,
    trusted: Vec<behind_proxy::Cidr>,
    addr: impl Into<SocketAddr>,
) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let route = route.with(behind_proxy::layer(Arc::new(trusted)));
    Server::new(TcpListener::bind(addr.into())).run(route).await
}

/// `app` を RPoem 上で起動し、サーバーが終わるまで待つ(通常は戻らない)。
pub async fn serve(app: Arc<App>, addr: impl Into<SocketAddr>) -> std::io::Result<()> {
    let (_addr, handle) = start(app, addr).await?;
    handle.await.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reaxt::react::VNode;
    use reaxt::RenderMode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn reaxt_app_runs_on_rpoem_with_one_call() {
        let app = Arc::new(
            App::new("rporea")
                .page("index.rs", RenderMode::Ssr, |_| VNode::element("h1").child(VNode::text("RPoRea")).build())
                .finish(),
        );
        let (addr, _h) = start(app, ([127, 0, 0, 1], 0)).await.unwrap();
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET / HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n").await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        assert!(out.contains("<h1>RPoRea</h1>"), "{out}");
    }
}

#[cfg(test)]
mod proxy_tests {
    use super::*;
    use crate::poem::{get, handler_fn, Route};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn call(trusted: &str, extra: &str) -> String {
        let h = handler_fn(|req, _| async move {
            let g = |k: &str| req.headers().get(k).and_then(|v| v.to_str().ok()).unwrap_or("-").to_string();
            let leaked = req.headers().keys().any(|k| k.as_str().starts_with("x-forwarded-"));
            let b = format!("client={} proto={} host={} prefix={} leaked={}", g("x-rporea-client-ip"), g("x-rporea-proto"), g("host"), g("x-rporea-prefix"), leaked);
            crate::poem::hyper_compat::Response::new(crate::poem::fixed_body(b.into()))
        });
        let route = Route::new().at("/", get(h));
        let (addr, _h) = start_route(route, behind_proxy::Cidr::parse_list(trusted).unwrap(), ([127, 0, 0, 1], 0)).await.unwrap();
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(format!("GET / HTTP/1.1\r\nhost: inner\r\n{extra}connection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    const FWD: &str = "x-forwarded-for: 9.9.9.9\r\nx-forwarded-proto: https\r\nx-forwarded-host: example.com\r\nx-forwarded-prefix: /app\r\n";

    #[tokio::test]
    async fn untrusted_peer_cannot_spoof_forwarded_headers() {
        let out = call("", FWD).await;
        assert!(out.contains("client=127.0.0.1 proto=http host=inner prefix=- leaked=false"), "{out}");
    }

    #[tokio::test]
    async fn trusted_proxy_headers_are_resolved_in_serve() {
        let out = call("127.0.0.1", FWD).await;
        assert!(out.contains("client=9.9.9.9 proto=https host=example.com prefix=/app leaked=false"), "{out}");
    }
}
