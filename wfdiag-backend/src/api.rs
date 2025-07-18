use actix_web::{web, HttpResponse, Result};
use crate::models::*;
use crate::service::DiagnosticService;
use uuid::Uuid;

pub async fn get_system_info() -> Result<HttpResponse> {
    let is_admin = crate::admin::is_running_as_admin();
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();

    let system_info = SystemInfo {
        os_version: sysinfo::System::long_os_version().unwrap_or_else(|| "Unknown".to_string()),
        computer_name: sysinfo::System::host_name().unwrap_or_else(|| "Unknown".to_string()),
        username: whoami::username(),
        is_admin,
        cpu_info: sys.cpus().first()
            .map(|cpu| cpu.brand().to_string())
            .unwrap_or_default(),
        total_memory_gb: sys.total_memory() as f64 / 1_073_741_824.0,
        available_memory_gb: sys.available_memory() as f64 / 1_073_741_824.0,
    };

    Ok(HttpResponse::Ok().json(ApiResponse::success(system_info)))
}

pub async fn get_tasks(
    service: web::Data<DiagnosticService>,
) -> Result<HttpResponse> {
    let tasks = service.get_available_tasks().await;
    Ok(HttpResponse::Ok().json(ApiResponse::success(tasks)))
}

pub async fn start_diagnostics(
    service: web::Data<DiagnosticService>,
    request: web::Json<DiagnosticRequest>,
) -> Result<HttpResponse> {
    match service.start_diagnostics(request.into_inner()).await {
        Ok(session) => Ok(HttpResponse::Ok().json(ApiResponse::success(session))),
        Err(e) => Ok(HttpResponse::BadRequest().json(ApiResponse::<()>::error(e.to_string()))),
    }
}

pub async fn get_session_status(
    service: web::Data<DiagnosticService>,
    session_id: web::Path<Uuid>,
) -> Result<HttpResponse> {
    match service.get_session(session_id.into_inner()).await {
        Some(session) => Ok(HttpResponse::Ok().json(ApiResponse::success(session))),
        None => Ok(HttpResponse::NotFound().json(ApiResponse::<()>::error("Session not found".to_string()))),
    }
}

pub async fn cancel_session(
    service: web::Data<DiagnosticService>,
    session_id: web::Path<Uuid>,
) -> Result<HttpResponse> {
    match service.cancel_session(session_id.into_inner()).await {
        Ok(_) => Ok(HttpResponse::Ok().json(ApiResponse::success("Session cancelled".to_string()))),
        Err(e) => Ok(HttpResponse::BadRequest().json(ApiResponse::<()>::error(e.to_string()))),
    }
}

pub async fn download_results(
    session_id: web::Path<Uuid>,
) -> Result<HttpResponse> {
    let desktop_path = dirs::desktop_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let zip_path = desktop_path.join(format!("WF-Diag_{}.zip", session_id));

    if zip_path.exists() {
        Ok(HttpResponse::Ok()
            .content_type("application/zip")
            .insert_header((
                "Content-Disposition",
                format!("attachment; filename=\"WF-Diag_{}.zip\"", session_id),
            ))
            .body(std::fs::read(&zip_path).unwrap_or_default()))
    } else {
        Ok(HttpResponse::NotFound().json(
            ApiResponse::<()>::error("Results file not found".to_string())
        ))
    }
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/v1")
            .route("/system", web::get().to(get_system_info))
            .route("/tasks", web::get().to(get_tasks))
            .route("/diagnostics", web::post().to(start_diagnostics))
            .route("/diagnostics/{session_id}", web::get().to(get_session_status))
            .route("/diagnostics/{session_id}/cancel", web::post().to(cancel_session))
            .route("/diagnostics/{session_id}/download", web::get().to(download_results))
    );
}