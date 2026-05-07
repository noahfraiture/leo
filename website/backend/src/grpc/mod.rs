pub mod healthz {
    tonic::include_proto!("healthz.v1");
}
use healthz::healthz_service_server::{HealthzService, HealthzServiceServer};
use healthz::{CheckRequest, CheckResponse};
use tonic::transport::Server;
use tonic::{Request, Response, Status};

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:8081".parse().unwrap();
    let healthz = Healthz {};
    let svc = HealthzServiceServer::new(healthz);
    Server::builder().add_service(svc).serve(addr).await?;
    Ok(())
}

struct Healthz;

#[tonic::async_trait]
impl HealthzService for Healthz {
    async fn check(
        &self,
        _request: Request<CheckRequest>,
    ) -> Result<Response<CheckResponse>, Status> {
        Ok(Response::new(CheckResponse {
            status: String::from("success"),
        }))
    }
}
