use serde_json::Value;
use socketioxide::{extract::{AckSender, Bin, Data, SocketRef, State, TryData}, socket::DisconnectReason};

use crate::{domain::player::{RsPlayer, RsPlayerActionRequest, RsPlayerPlayRequest}, model::{server::AuthMessage, users::UserRole, ModelController}, tools::log::{log_error, log_info, LogServiceType}};

use super::mw_auth::parse_auth_message;


pub async fn on_connect(socket: SocketRef, mc: ModelController, data: Result<AuthMessage, serde_json::Error>) {
    if let Ok(auth) = data {
        let auth = parse_auth_message(&auth, &mc).await;
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
                crate::model::users::ConnectedUser::Anonymous | crate::model::users::ConnectedUser::Guest(_) => {},
                crate::model::users::ConnectedUser::ServerAdmin => {},
                crate::model::users::ConnectedUser::Share(_) => {},
                crate::model::users::ConnectedUser::UploadKey(_) => todo!(),
            }    

            socket.emit("auth", auth.clone()).ok();
            mc.send_players_to_socket(&socket, &auth).await;

            let mc_player = mc.clone();
            let auth_player = auth.clone();
            //println!("mcplayer {:?}", mc_player.io);
            socket.on(
                "player-available",
                move |socket: SocketRef, Data::<RsPlayer>(data)| {
                    //println!("mcplayer2 {:?}", mc_player.io);
                    tokio::spawn(async move {
                        
                        //println!("mcplayer3 {:?}", mc_player.io);
                        let added = mc_player.add_player(data.clone(), socket, &auth_player).await;
                        if let Err(added) = added {
                            log_error(LogServiceType::Other, format!("Error adding player: {} ({}) => {:?}", data.name, data.player, added)) 
                        } else {
                            log_info(LogServiceType::Other, format!("Successfully added player: {} ({})", data.name, data.player)) 
                        }
                    });
                },
            );

            let mc_disco = mc.clone();
            socket.on_disconnect(|socket: SocketRef, reason: DisconnectReason| async move { 
                println!("Socket {} on ns {} disconnected, reason: {:?}", socket.id, socket.ns(), reason); 
                match mc_disco.remove_player(socket.id.to_string()).await {
                    Ok(_) => (),
                    Err(err) => log_error(LogServiceType::Other, format!("Error removing player => {:?}", err)) ,
                };

            });

            let mc_disco = mc.clone();
            let socket_id = socket.id.to_string();
            let auth_player = auth.clone();
            socket.on("player-request",|Data::<RsPlayerPlayRequest>(data)| async move { 
                let added =  mc_disco.send_play_request(data.clone(), &auth_player).await;
                        if let Err(added) = added {
                            log_error(LogServiceType::Other, format!("Error seding player request: {:?} => {:?}", data, added)) 
                        } else {
                            log_info(LogServiceType::Other, format!("Successfully send player request: {:?}", data)) 
                        }
                match mc_disco.remove_player(socket_id).await {
                    Ok(_) => (),
                    Err(err) => log_error(LogServiceType::Other, format!("Error removing player => {:?}", err)) ,
                };
            });

            let mc_disco = mc.clone();
            let auth_player = auth.clone();
            socket.on("player-action",|Data::<RsPlayerActionRequest>(data)| async move { 
                let added =  mc_disco.send_play_action(data.clone(), &auth_player).await;
                        if let Err(added) = added {
                            log_error(LogServiceType::Other, format!("Error seding player action: {:?} => {:?}", data, added)) 
                        } else {
                            log_info(LogServiceType::Other, format!("Successfully send player action: {:?}", data)) 
                        }

            });

            
        } else {
            socket.disconnect().ok();
        }
    } else {
        socket.disconnect().ok();
    }
}
