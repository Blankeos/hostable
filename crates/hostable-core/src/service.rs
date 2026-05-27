use crate::registry::ServerRecord;

pub fn game_service_name(server: &ServerRecord) -> String {
    format!("hostable-{}.service", server.id.0)
}
