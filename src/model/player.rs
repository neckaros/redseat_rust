use socketioxide::{extract::SocketRef, socket::Socket};

use crate::{domain::player::{RsPlayer, RsPlayerActionRequest, RsPlayerAvailable, RsPlayerEvent, RsPlayerPlayRequest}, error::RsResult, tools::log::{log_error, LogServiceType}};

use super::{users::ConnectedUser, ModelController};



impl ModelController {

    pub async fn send_players_to_socket(&self, socket: &SocketRef, user: &ConnectedUser) {
        let players = self.list_players(user).await;
        if let Ok(players) = players {
            let message = players.into_iter().map(RsPlayerEvent::from).collect::<Vec<_>>();
            let _ = socket.emit("players-list", [message]);
            
        }

	}

    pub async fn send_players(&self, players: Vec<RsPlayerAvailable>) {
        let message = players.into_iter().map(RsPlayerEvent::from).collect::<Vec<_>>();
        self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_role(&super::users::UserRole::Read);
			if r.is_ok() {
				let _ = socket.emit("players-list", [message]);
			} else {
                log_error(LogServiceType::Source, format!("Unable to emit player list to {:?} (socket: {}): {:?}", user, socket.id, r))
            }
		});
	}

    pub async fn send_play_request(&self, request: RsPlayerPlayRequest, user: &ConnectedUser) -> RsResult<()> {
        let players = self.list_players(user).await?;
		let player = players.into_iter().find(|p| p.socket.id.to_string() == request.id).ok_or(crate::Error::NotFound)?;
        player.socket.emit("player-request", request).map_err(|_| crate::Error::Error("Unable to send play request".to_string()))?;
        Ok(())
	}

    pub async fn send_play_action(&self, request: RsPlayerActionRequest, user: &ConnectedUser) -> RsResult<()> {
        let players = self.list_players(user).await?;
		let player = players.into_iter().find(|p| p.socket.id.to_string() == request.id).ok_or(crate::Error::NotFound)?;
        player.socket.emit("player-action", request.action).map_err(|_| crate::Error::Error("Unable to send play request".to_string()))?;
        Ok(())
	}

    pub async fn list_players(&self, user: &ConnectedUser) -> RsResult<Vec<RsPlayerAvailable>> {
        user.check_role(&super::users::UserRole::Read)?;
        let players = self.players.read().await;
        let players = players.clone();
        Ok(players)
    }

    pub async fn add_player(&self, player: RsPlayer, socket: SocketRef, user: &ConnectedUser)  -> RsResult<()> {
        user.check_role(&super::users::UserRole::Read)?;
        let mut players = self.players.write().await;
        let player = RsPlayerAvailable {
            socket,
            player,
        };
        if !players.iter().any(|p| player.socket.id == p.socket.id  && p.player.name == player.player.name ) {
            println!("added player {}, {}", player.socket.id, player.player.name);
            players.push(player);
            self.send_players(players.clone()).await;
        }
        
        Ok(())
    }
    pub async fn remove_player(&self, socket_id: String)  -> RsResult<()> {
        let mut players = self.players.write().await;
        if let Some(player) = players.iter().position(|p| socket_id == p.socket.id.to_string() ) {
            println!("removed player");
            players.remove(player);
            self.send_players(players.clone()).await;
        }
        
        Ok(())
    }


}