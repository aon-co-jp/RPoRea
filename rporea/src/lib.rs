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
pub async fn start(app: Arc<App>, addr: impl Into<SocketAddr>) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    Server::new(TcpListener::bind(addr.into())).run(reaxt_rpoem::route_production(app)).await
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
