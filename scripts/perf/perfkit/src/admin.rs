//! 被测进程的旁路 admin 端口。
//!
//! 单独开一个端口而不是挂在被测路由上，是因为 `/v1/*` 前缀会被 access +
//! hold 两层中间件拦截并计费 —— admin 请求要是走那条路，就会把自己的开销
//! 算进被测样本里。

use std::net::SocketAddr;

use axum::Router;
use axum::routing::{get, post};

use crate::counting_alloc;

/// 起 admin 服务：
/// * `GET  /stats`  当前分配计数（先取快照再建响应，尽量少混进自己的分配）
/// * `POST /reset`  清零
/// * `GET  /health` 存活探针
pub async fn serve(addr: SocketAddr) -> anyhow::Result<()> {
    let app = Router::new()
        .route(
            "/stats",
            get(|| async {
                let snap = counting_alloc::snapshot();
                axum::Json(snap)
            }),
        )
        .route(
            "/reset",
            post(|| async {
                counting_alloc::reset();
                "ok"
            }),
        )
        .route("/health", get(|| async { "ok" }));

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
