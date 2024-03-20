use serde_json::Value;
use socketioxide::extract::{AckSender, Bin, Data, SocketRef, State, TryData};

use crate::model::{server::AuthMessage, users::UserRole, ModelController};

use super::mw_auth::parse_auth_message;


pub async fn on_connect(socket: SocketRef, State(mc): State<ModelController>, TryData(data): TryData<AuthMessage>) {
    if let Ok(auth) = data {
        let auth = parse_auth_message(&auth, mc).await;
            if let Ok(auth) = auth {
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
                crate::model::users::ConnectedUser::Anonymous => {},
                crate::model::users::ConnectedUser::ServerAdmin => {},
                crate::model::users::ConnectedUser::Share(_) => {},
            }    
            socket.emit("auth", auth.clone()).ok();

            socket.on(
                "message",
                |socket: SocketRef, Data::<Value>(data), Bin(bin)| {
                    socket.bin(bin).emit("message-back", data).ok();
                },
            );

            socket.on(
                "message-with-ack",
                |Data::<Value>(data), ack: AckSender, Bin(bin)| {
                    ack.bin(bin).send(data).ok();
                },
            );
        } else {
            socket.disconnect().ok();
        }
    } else {
        socket.disconnect().ok();
    }
}
