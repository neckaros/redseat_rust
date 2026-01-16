use socketioxide::{extract::{SocketRef}, socket::DisconnectReason};

use crate::model::{server::AuthMessage, users::UserRole, ModelController};

use super::mw_auth::parse_auth_message;


pub async fn on_connect(socket: SocketRef, mc: ModelController, data: Result<AuthMessage, serde_json::Error>) {
    println!("Socket {} on ns {} trying to connect", socket.id, socket.ns());
    if let Ok(auth) = data {
        let auth = parse_auth_message(&auth, &mc).await;
            if let Ok(auth) = auth {
                println!("Socket {} on ns {} authenticated", socket.id, socket.ns());
                socket.extensions.insert(auth.clone());
                match &auth {
                    crate::model::users::ConnectedUser::Server(user) => {
                        for library in &user.libraries {
                            let _ = socket.join(format!("lib:{}", library.id));
                        }
                        if user.role == UserRole::Admin {
                            let _ = socket.join("admin");
                        }
                    },
                    crate::model::users::ConnectedUser::Anonymous | crate::model::users::ConnectedUser::Guest(_) => {},
                    crate::model::users::ConnectedUser::ServerAdmin => {},
                    crate::model::users::ConnectedUser::Share(_) => {},
                    crate::model::users::ConnectedUser::UploadKey(_) => todo!(),
                }

                socket.emit("auth", auth.clone()).ok();

                socket.on_disconnect(|socket: SocketRef, reason: DisconnectReason| async move {
                    println!("Socket {} on ns {} disconnected, reason: {:?}", socket.id, socket.ns(), reason);
                });


            } else {
                socket.disconnect().ok();
            }
    } else {
        socket.disconnect().ok();
    }
}
