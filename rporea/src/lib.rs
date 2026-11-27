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

#[cfg(test)]
mod ws_tests {
    use super::*;
    use reaxt::app::{Params, WsEvent, WsOut};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// 本番向けの既定(トレース・圧縮・プロキシ層)を通しても、WebSocket が実ソケットで往復できる。
    #[tokio::test]
    async fn websocket_works_through_production_layers() {
        let app = Arc::new(
            App::new("rporea")
                .ws(
                    "echo/[who].rs",
                    Arc::new(|p: &Params, ev: &WsEvent| match ev {
                        WsEvent::Open => vec![WsOut::Send(format!("hello {}", p[0].1))],
                        WsEvent::Text(t) => vec![WsOut::Send(format!("echo {t}"))],
                        WsEvent::Close => vec![],
                    }),
                )
                .finish(),
        );
        let (addr, _h) = start(app, ([127, 0, 0, 1], 0)).await.unwrap();
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET /echo/ann HTTP/1.1\r\nhost: x\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\naccept-encoding: gzip\r\n\r\n").await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(s.read_u8().await.unwrap());
        }
        assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 101"), "{}", String::from_utf8_lossy(&head));
        let mut h = [0u8; 2];
        s.read_exact(&mut h).await.unwrap();
        let mut b = vec![0u8; (h[1] & 0x7f) as usize];
        s.read_exact(&mut b).await.unwrap();
        assert_eq!(String::from_utf8(b).unwrap(), "hello ann");
        let k = [9u8, 8, 7, 6];
        let msg = b"ping";
        let mut f = vec![0x81, 0x80 | msg.len() as u8];
        f.extend_from_slice(&k);
        f.extend(msg.iter().enumerate().map(|(i, b)| b ^ k[i % 4]));
        s.write_all(&f).await.unwrap();
        s.read_exact(&mut h).await.unwrap();
        let mut b = vec![0u8; (h[1] & 0x7f) as usize];
        s.read_exact(&mut b).await.unwrap();
        assert_eq!(String::from_utf8(b).unwrap(), "echo ping");
    }
}
