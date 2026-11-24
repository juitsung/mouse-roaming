use mouseroaming_proto::{Edge, KeyCode, Message, MouseButton, OsKind, ScreenRect};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveRemote {
    pub peer_id: String,
    pub edge: Edge,
    pub os: OsKind,
}

#[derive(Debug)]
pub struct EdgeRouter {
    local_screen: ScreenRect,
    dead_corner_px: i32,
    cooldown: Duration,
    last_switch_at: Option<Instant>,
    bindings: HashMap<Edge, String>,
    active_remote: Option<ActiveRemote>,
    pressed_keys: HashSet<KeyCode>,
    pressed_buttons: HashSet<MouseButton>,
}

impl EdgeRouter {
    pub fn new(local_screen: ScreenRect, dead_corner_px: i32, cooldown_ms: u64) -> Self {
        Self {
            local_screen,
            dead_corner_px: dead_corner_px.max(0),
            cooldown: Duration::from_millis(cooldown_ms),
            last_switch_at: None,
            bindings: HashMap::new(),
            active_remote: None,
            pressed_keys: HashSet::new(),
            pressed_buttons: HashSet::new(),
        }
    }

    pub fn update_screen(&mut self, screen: ScreenRect) {
        self.local_screen = screen;
    }

    pub fn local_screen(&self) -> ScreenRect {
        self.local_screen
    }

    pub fn set_dead_corner_px(&mut self, px: i32) {
        self.dead_corner_px = px.max(0);
    }

    pub fn set_cooldown_ms(&mut self, ms: u64) {
        self.cooldown = Duration::from_millis(ms);
    }

    pub fn bind_edge(&mut self, edge: Edge, peer_id: String) {
        // Ensure a peer is only bound to one edge at a time
        self.bindings.retain(|_, v| v != &peer_id);
        self.bindings.insert(edge, peer_id);
    }

    /// Move a peer to `target_edge`. If `target_edge` is already occupied by another peer
    /// and `peer_id` previously occupied `old_edge`, swap the two peers' edges atomically.
    pub fn move_or_swap_peer_to_edge(&mut self, peer_id: String, target_edge: Edge) {
        let old_edge = self.edge_for_peer(&peer_id);
        if old_edge == Some(target_edge) {
            return;
        }
        let displaced_peer = self.bindings.remove(&target_edge);
        self.bindings.retain(|_, v| v != &peer_id);
        self.bindings.insert(target_edge, peer_id);
        if let (Some(prev_edge), Some(other_id)) = (old_edge, displaced_peer) {
            self.bindings.insert(prev_edge, other_id);
        }
    }

    pub fn unbind_edge(&mut self, edge: Edge) {
        self.bindings.remove(&edge);
    }

    pub fn unbind_peer(&mut self, peer_id: &str) {
        self.bindings.retain(|_, v| v != peer_id);
    }

    pub fn edge_for_peer(&self, peer_id: &str) -> Option<Edge> {
        self.bindings
            .iter()
            .find_map(|(e, id)| if id == peer_id { Some(*e) } else { None })
    }

    pub fn peer_on_edge(&self, edge: Edge) -> Option<&str> {
        self.bindings.get(&edge).map(String::as_str)
    }

    pub fn bindings(&self) -> &HashMap<Edge, String> {
        &self.bindings
    }

    pub fn active_remote(&self) -> Option<&ActiveRemote> {
        self.active_remote.as_ref()
    }

    pub fn is_remote_active(&self) -> bool {
        self.active_remote.is_some()
    }

    fn in_cooldown(&self, now: Instant) -> bool {
        self.last_switch_at
            .map(|t| now.duration_since(t) < self.cooldown)
            .unwrap_or(false)
    }

    fn inside_dead_corner(&self, edge: Edge, x: i32, y: i32) -> bool {
        let m = self.dead_corner_px;
        if m <= 0 {
            return false;
        }
        match edge {
            Edge::Left | Edge::Right => {
                y < self.local_screen.y + m || y > self.local_screen.bottom() - m
            }
            Edge::Top | Edge::Bottom => {
                x < self.local_screen.x + m || x > self.local_screen.right() - m
            }
        }
    }

    /// Check if local cursor movement crosses a bound screen edge.
    /// Returns `Some((edge, peer_id, ratio))`.
    pub fn check_local_edge_crossing(
        &self,
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
        now: Instant,
    ) -> Option<(Edge, String, f32)> {
        if self.active_remote.is_some() || self.in_cooldown(now) {
            return None;
        }

        let hit_edge = if x <= self.local_screen.x && dx <= 0 {
            Some(Edge::Left)
        } else if x >= self.local_screen.right() && dx >= 0 {
            Some(Edge::Right)
        } else if y <= self.local_screen.y && dy <= 0 {
            Some(Edge::Top)
        } else if y >= self.local_screen.bottom() && dy >= 0 {
            Some(Edge::Bottom)
        } else {
            None
        }?;

        if self.inside_dead_corner(hit_edge, x, y) {
            return None;
        }

        let peer_id = self.bindings.get(&hit_edge)?.clone();
        let ratio = self.local_screen.edge_ratio(hit_edge, x, y);
        Some((hit_edge, peer_id, ratio))
    }

    pub fn enter_remote(&mut self, peer_id: String, edge: Edge, os: OsKind, now: Instant) {
        self.active_remote = Some(ActiveRemote { peer_id, edge, os });
        self.last_switch_at = Some(now);
        self.pressed_keys.clear();
        self.pressed_buttons.clear();
    }

    pub fn track_remote_key(&mut self, key: KeyCode, pressed: bool) {
        if pressed {
            self.pressed_keys.insert(key);
        } else {
            self.pressed_keys.remove(&key);
        }
    }

    pub fn track_remote_button(&mut self, button: MouseButton, pressed: bool) {
        if pressed {
            self.pressed_buttons.insert(button);
        } else {
            self.pressed_buttons.remove(&button);
        }
    }

    /// Return control to local host and produce cleanup release messages for any stuck remote inputs.
    pub fn return_local(&mut self, now: Instant) -> (Option<ActiveRemote>, Vec<Message>) {
        let prev = self.active_remote.take();
        self.last_switch_at = Some(now);

        let mut cleanup = Vec::new();
        for key in self.pressed_keys.drain() {
            cleanup.push(Message::KeyEvent {
                key,
                pressed: false,
            });
        }
        for button in self.pressed_buttons.drain() {
            cleanup.push(Message::MouseButton {
                button,
                pressed: false,
            });
        }
        if prev.is_some() {
            cleanup.push(Message::ReleaseAll);
        }
        (prev, cleanup)
    }

    /// Client-side helper: checks whether an incoming relative delta pushes the client cursor
    /// past `return_edge` (the edge where the Server sits relative to the Client).
    pub fn check_client_return_edge(
        screen: ScreenRect,
        return_edge: Edge,
        cur_x: i32,
        cur_y: i32,
        dx: i32,
        dy: i32,
        dead_corner_px: i32,
    ) -> Option<f32> {
        let nx = cur_x + dx;
        let ny = cur_y + dy;
        let crossed = match return_edge {
            Edge::Left => nx < screen.x && dx < 0,
            Edge::Right => nx > screen.right() && dx > 0,
            Edge::Top => ny < screen.y && dy < 0,
            Edge::Bottom => ny > screen.bottom() && dy > 0,
        };
        if !crossed {
            return None;
        }
        if dead_corner_px > 0 {
            let in_corner = match return_edge {
                Edge::Left | Edge::Right => {
                    cur_y < screen.y + dead_corner_px || cur_y > screen.bottom() - dead_corner_px
                }
                Edge::Top | Edge::Bottom => {
                    cur_x < screen.x + dead_corner_px || cur_x > screen.right() - dead_corner_px
                }
            };
            if in_corner {
                return None;
            }
        }
        Some(screen.edge_ratio(return_edge, cur_x, cur_y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_crossing_respects_dead_corner_and_cooldown() {
        let screen = ScreenRect::new(0, 0, 1920, 1080);
        let mut router = EdgeRouter::new(screen, 30, 200);
        router.bind_edge(Edge::Right, "mac-1".into());
        let t0 = Instant::now();

        // Top-right dead corner (y=10 < 30) must NOT switch
        assert!(router
            .check_local_edge_crossing(1919, 10, 5, 0, t0)
            .is_none());

        // Middle of right edge switches cleanly
        let hit = router
            .check_local_edge_crossing(1919, 540, 5, 0, t0)
            .expect("should cross right edge");
        assert_eq!(hit.0, Edge::Right);
        assert_eq!(hit.1, "mac-1");
        assert!((hit.2 - 0.5).abs() < 0.01);

        router.enter_remote("mac-1".into(), Edge::Right, OsKind::MacOS, t0);
        router.track_remote_key(KeyCode::LeftMeta, true);
        router.track_remote_button(MouseButton::Left, true);

        // Returning releases all stuck keys/buttons + sends ReleaseAll
        let (prev, cleanup) = router.return_local(t0 + Duration::from_millis(50));
        assert!(prev.is_some());
        assert!(cleanup.contains(&Message::KeyEvent {
            key: KeyCode::LeftMeta,
            pressed: false
        }));
        assert!(cleanup.contains(&Message::MouseButton {
            button: MouseButton::Left,
            pressed: false
        }));
        assert!(cleanup.contains(&Message::ReleaseAll));

        // Immediate re-crossing during cooldown must be blocked
        assert!(router
            .check_local_edge_crossing(1919, 540, 5, 0, t0 + Duration::from_millis(100))
            .is_none());

        // Crossing after cooldown succeeds
        assert!(router
            .check_local_edge_crossing(1919, 540, 5, 0, t0 + Duration::from_millis(300))
            .is_some());
    }

    #[test]
    fn client_return_edge_triggers_on_overshoot() {
        let mac_screen = ScreenRect::new(0, 0, 1440, 900);
        // Client sits to the Right of Server, so return edge is Left
        assert!(EdgeRouter::check_client_return_edge(
            mac_screen,
            Edge::Left,
            2,
            450,
            -5,
            0,
            24
        )
        .is_some());

        // Moving right away from left edge does NOT trigger return
        assert!(EdgeRouter::check_client_return_edge(
            mac_screen,
            Edge::Left,
            2,
            450,
            5,
            0,
            24
        )
        .is_none());
    }

    #[test]
    fn move_or_swap_peer_swaps_occupied_edges() {
        let screen = ScreenRect::new(0, 0, 1920, 1080);
        let mut router = EdgeRouter::new(screen, 24, 150);
        router.bind_edge(Edge::Left, "mac-a".into());
        router.bind_edge(Edge::Right, "win-b".into());

        // Dragging mac-a onto Right swaps it with win-b
        router.move_or_swap_peer_to_edge("mac-a".into(), Edge::Right);
        assert_eq!(router.peer_on_edge(Edge::Right), Some("mac-a"));
        assert_eq!(router.peer_on_edge(Edge::Left), Some("win-b"));

        // Dragging mac-a onto empty Top moves it and vacates Right
        router.move_or_swap_peer_to_edge("mac-a".into(), Edge::Top);
        assert_eq!(router.peer_on_edge(Edge::Top), Some("mac-a"));
        assert_eq!(router.peer_on_edge(Edge::Right), None);
        assert_eq!(router.peer_on_edge(Edge::Left), Some("win-b"));

        // Dragging an unbound peer onto occupied Left replaces Left cleanly
        router.move_or_swap_peer_to_edge("mac-c".into(), Edge::Left);
        assert_eq!(router.peer_on_edge(Edge::Left), Some("mac-c"));
        assert_eq!(router.edge_for_peer("win-b"), None);
    }
}
