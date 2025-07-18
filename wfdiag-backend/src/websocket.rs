use actix_web::{web, Error, HttpRequest, HttpResponse};
use actix_ws::Message;
use futures_util::StreamExt;
use log::{info, error, debug};

use crate::models::ProgressUpdate;

pub async fn websocket_handler(
    req: HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, Error> {
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, stream)?;
    
    info!("New WebSocket connection established");

    // For now, just handle the connection without broadcasting
    // In production, you'd integrate with a proper pub/sub system
    actix_web::rt::spawn(async move {
        loop {
            tokio::select! {
                // Handle incoming messages from client
                msg = msg_stream.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            debug!("Received text message: {}", text);
                            // Echo back for now
                            if session.text(format!("Echo: {}", text)).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("WebSocket connection closed by client");
                            break;
                        }
                        Some(Ok(Message::Ping(bytes))) => {
                            if session.pong(&bytes).await.is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let _ = session.close(None).await;
        info!("WebSocket connection closed");
    });

    Ok(response)
}

pub fn configure_websocket(cfg: &mut web::ServiceConfig) {
    cfg.route("/ws", web::get().to(websocket_handler));
}