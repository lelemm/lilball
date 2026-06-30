use approx::assert_relative_eq;
use fidget_sim::{
    Ball, BottomEdge, Bounds, FIXED_DT, InteractionState, ParticleSystem, Trail, World, WorldConfig,
};
use glam::{Vec2, Vec4};

fn no_gravity_world() -> World {
    let cfg = WorldConfig {
        gravity: Vec2::ZERO,
        ..WorldConfig::default()
    };
    World::new(cfg, Bounds::new(0.0, 0.0, 1000.0, 600.0))
}

#[test]
fn ball_starts_centered() {
    let world = no_gravity_world();
    assert_relative_eq!(world.ball.pos.x, 500.0);
    assert_relative_eq!(world.ball.pos.y, 300.0);
}

#[test]
fn world_uses_configured_ball_and_spring_size() {
    let bounds = Bounds::new(0.0, 0.0, 1000.0, 600.0);
    let large = World::new(WorldConfig::default(), bounds);
    let cfg = WorldConfig {
        ball_radius: 24.0,
        spring_interaction_scale: 0.62,
        spring_length_scale: 0.55,
        ..WorldConfig::default()
    };
    let world = World::new(cfg, bounds);

    assert_relative_eq!(world.ball.radius, 24.0);
    assert!(world.spring.intersection_capture_radius < 90.0);
    assert!(world.spring.entangle_capture_radius < 60.0);
    assert!(world.spring.rest_length < large.spring.rest_length * 0.7);
    assert!(
        world.ball.pos.y < large.ball.pos.y,
        "shorter spring should hang higher, small={:?}, large={:?}",
        world.ball.pos,
        large.ball.pos
    );
}

#[test]
fn ball_bounces_off_right_wall_and_reverses() {
    let mut world = no_gravity_world();
    world.ball.vel = Vec2::new(2000.0, 0.0);
    world.ball.pos = Vec2::new(900.0, 300.0);
    let restitution = world.ball.restitution;

    let mut bounced = false;
    for _ in 0..240 {
        world.advance(FIXED_DT);
        if world.ball.vel.x < 0.0 {
            bounced = true;
            break;
        }
    }
    assert!(
        bounced,
        "ball should reverse x velocity after hitting right wall"
    );
    // Energy lost to restitution => speed reduced.
    assert!(world.ball.speed() <= 2000.0 * restitution + 1.0);
}

#[test]
fn ball_never_escapes_bounds() {
    let mut world = no_gravity_world();
    world.ball.vel = Vec2::new(4000.0, -3500.0);
    let r = world.ball.radius;
    for _ in 0..2000 {
        world.advance(FIXED_DT);
        assert!(world.ball.pos.x >= world.bounds.left + r - 0.6);
        assert!(world.ball.pos.x <= world.bounds.right - r + 0.6);
        assert!(world.ball.pos.y >= world.bounds.top + r - 0.6);
        assert!(world.ball.pos.y <= world.bounds.bottom - r + 0.6);
    }
}

#[test]
fn gravity_pulls_ball_down() {
    let mut world = World::new(
        WorldConfig::default(),
        Bounds::new(0.0, 0.0, 1000.0, 6000.0),
    );
    world.ball.pos = Vec2::new(500.0, 100.0);
    world.ball.vel = Vec2::ZERO;
    let y0 = world.ball.pos.y;
    for _ in 0..30 {
        world.advance(FIXED_DT);
    }
    assert!(
        world.ball.pos.y > y0,
        "gravity should increase y (downward)"
    );
    assert!(world.ball.vel.y > 0.0);
}

#[test]
fn attached_spring_pulls_ball_toward_rest_position() {
    let mut world = no_gravity_world();
    let rest = world.spring.rest_position();
    world.ball.pos = rest + Vec2::new(0.0, 160.0);
    world.ball.vel = Vec2::ZERO;

    world.advance(FIXED_DT);

    assert!(world.spring_attached());
    assert!(
        world.ball.vel.y < 0.0,
        "spring should pull upward when stretched below rest, vel={:?}",
        world.ball.vel
    );
}

#[test]
fn stretched_attached_spring_rebounds_past_rest() {
    let mut world = no_gravity_world();
    let rest = world.spring.rest_position();
    world.ball.pos = rest + Vec2::new(0.0, 220.0);
    world.ball.vel = Vec2::ZERO;

    let mut overshot = false;
    for _ in 0..180 {
        world.advance(FIXED_DT);
        if world.ball.pos.y < rest.y - 8.0 {
            overshot = true;
            break;
        }
    }

    assert!(
        overshot,
        "stretched spring should keep enough energy to rebound past rest, pos={:?}, rest={:?}",
        world.ball.pos, rest
    );
}

#[test]
fn hook_defaults_above_screen_top() {
    let world = no_gravity_world();
    assert!(
        world.spring.anchor.y < world.bounds.top,
        "hook should default off-screen above the desktop, anchor={:?}",
        world.spring.anchor
    );
}

#[test]
fn hud_parameter_setters_clamp_and_apply() {
    let mut world = no_gravity_world();

    world.set_gravity_strength(1800.0);
    world.set_spring_stiffness(320.0);
    world.set_spring_damping(44.0);
    world.set_hook_offset_y(-300.0);

    assert_relative_eq!(world.gravity_strength(), 1800.0);
    assert_relative_eq!(world.spring_stiffness(), 320.0);
    assert_relative_eq!(world.spring_damping(), 44.0);
    assert_relative_eq!(world.hook_offset_y(), -300.0);
    assert!(world.spring.anchor.y < world.bounds.top);
}

#[test]
fn cut_spring_lets_ball_fall_through_bottom() {
    let mut world = World::new(WorldConfig::default(), Bounds::new(0.0, 0.0, 1000.0, 600.0));
    let r = world.ball.radius;
    world.cut_spring();
    world.ball.pos = Vec2::new(500.0, world.bounds.bottom - r - 1.0);
    world.ball.vel = Vec2::new(0.0, 900.0);

    for _ in 0..8 {
        world.advance(FIXED_DT);
    }

    assert!(!world.spring_attached());
    assert!(
        world.ball.pos.y + r > world.bounds.bottom,
        "cut spring should disable the bottom wall, pos={:?}",
        world.ball.pos
    );
}

#[test]
fn zero_recall_margin_hides_as_soon_as_ball_leaves_bottom() {
    let mut world = World::new(WorldConfig::default(), Bounds::new(0.0, 0.0, 1000.0, 600.0));
    let r = world.ball.radius;
    world.cut_spring();
    world.set_recall_margin(0.0);
    world.ball.pos = Vec2::new(500.0, world.bounds.bottom + r + 2.0);
    world.ball.vel = Vec2::new(0.0, 900.0);

    world.advance(FIXED_DT);

    assert!(!world.ball_visible());
    assert!(!world.spring_attached());
}

#[test]
fn cut_spring_bounces_off_bottom_when_bottom_bounce_enabled() {
    let cfg = WorldConfig {
        bounce_bottom_edge: true,
        ..WorldConfig::default()
    };
    let mut world = World::new(cfg, Bounds::new(0.0, 0.0, 1000.0, 600.0));
    let r = world.ball.radius;
    world.cut_spring();
    world.ball.pos = Vec2::new(500.0, world.bounds.bottom - r - 1.0);
    world.ball.vel = Vec2::new(0.0, 900.0);

    for _ in 0..8 {
        world.advance(FIXED_DT);
    }

    assert!(!world.spring_attached());
    assert!(
        world.ball.pos.y + r <= world.bounds.bottom + 0.6,
        "bottom bounce should keep detached ball in bounds, pos={:?}",
        world.ball.pos
    );
    assert!(
        world.ball.vel.y <= 0.0,
        "bottom bounce should reverse downward velocity, vel={:?}",
        world.ball.vel
    );
}

#[test]
fn bottom_bounce_uses_monitor_edge_under_ball() {
    let cfg = WorldConfig {
        bounce_bottom_edge: true,
        ..WorldConfig::default()
    };
    let mut world = World::new(cfg, Bounds::new(0.0, 0.0, 2000.0, 1200.0));
    world.set_bottom_edges([
        BottomEdge::new(0.0, 1000.0, 800.0),
        BottomEdge::new(1000.0, 2000.0, 1200.0),
    ]);
    let r = world.ball.radius;
    world.cut_spring();
    world.ball.pos = Vec2::new(500.0, 800.0 - r - 1.0);
    world.ball.vel = Vec2::new(0.0, 900.0);

    for _ in 0..8 {
        world.advance(FIXED_DT);
    }

    assert!(
        world.ball.pos.y + r <= 800.0 + 0.6,
        "ball should bounce at the left monitor bottom edge, pos={:?}",
        world.ball.pos
    );
    assert!(
        world.ball.vel.y <= 0.0,
        "monitor-edge bounce should reverse downward velocity, vel={:?}",
        world.ball.vel
    );
}

#[test]
fn bottom_bounce_ignores_monitor_edges_outside_ball_span() {
    let cfg = WorldConfig {
        bounce_bottom_edge: true,
        ..WorldConfig::default()
    };
    let mut world = World::new(cfg, Bounds::new(0.0, 0.0, 2000.0, 1200.0));
    world.set_bottom_edges([
        BottomEdge::new(0.0, 1000.0, 800.0),
        BottomEdge::new(1000.0, 2000.0, 1200.0),
    ]);
    let r = world.ball.radius;
    world.cut_spring();
    world.ball.pos = Vec2::new(1500.0, 900.0);
    world.ball.vel = Vec2::new(0.0, 500.0);

    for _ in 0..8 {
        world.advance(FIXED_DT);
    }

    assert!(
        world.ball.pos.y > 900.0,
        "right-side ball should keep falling instead of snapping to the left monitor edge, pos={:?}",
        world.ball.pos
    );
    assert!(
        world.ball.pos.y + r > 800.0 + 20.0,
        "left monitor edge should not affect the right monitor span, pos={:?}",
        world.ball.pos
    );
}

#[test]
fn bottom_bounce_passes_through_stacked_monitor_seam() {
    let cfg = WorldConfig {
        bounce_bottom_edge: true,
        ..WorldConfig::default()
    };
    let bounds = Bounds::new(0.0, 0.0, 1000.0, 1200.0);
    let monitors = [
        Bounds::new(0.0, 0.0, 1000.0, 700.0),
        Bounds::new(0.0, 700.0, 1000.0, 1200.0),
    ];
    let edges = BottomEdge::exposed_from_bounds(&monitors, bounds);
    assert_eq!(edges, vec![BottomEdge::new(0.0, 1000.0, 1200.0)]);

    let mut world = World::new(cfg, bounds);
    world.set_bottom_edges(edges);
    let r = world.ball.radius;
    world.cut_spring();
    world.ball.pos = Vec2::new(500.0, 700.0 - r - 1.0);
    world.ball.vel = Vec2::new(0.0, 900.0);

    for _ in 0..8 {
        world.advance(FIXED_DT);
    }

    assert!(
        world.ball.pos.y + r > 700.0 + 8.0,
        "ball should pass through the seam into the lower monitor, pos={:?}",
        world.ball.pos
    );
    assert!(
        world.ball.vel.y > 0.0,
        "internal monitor seam should not reverse downward velocity, vel={:?}",
        world.ball.vel
    );
}

#[test]
fn exposed_bottom_edges_keep_uncovered_parts_of_upper_monitor() {
    let bounds = Bounds::new(0.0, 0.0, 1200.0, 1200.0);
    let monitors = [
        Bounds::new(0.0, 0.0, 1200.0, 700.0),
        Bounds::new(300.0, 700.0, 900.0, 1200.0),
    ];
    let edges = BottomEdge::exposed_from_bounds(&monitors, bounds);

    assert_eq!(
        edges,
        vec![
            BottomEdge::new(0.0, 300.0, 700.0),
            BottomEdge::new(900.0, 1200.0, 700.0),
            BottomEdge::new(300.0, 900.0, 1200.0),
        ]
    );
}

#[test]
fn fallen_ball_hides_in_pit_until_spawned() {
    let mut world = World::new(WorldConfig::default(), Bounds::new(0.0, 0.0, 1000.0, 600.0));
    world.cut_spring();
    world.ball.pos = Vec2::new(
        500.0,
        world.bounds.bottom + world.spring.recall_margin + world.ball.radius + 2.0,
    );
    world.ball.vel = Vec2::new(0.0, 1200.0);

    world.advance(FIXED_DT);

    assert!(!world.ball_visible());
    assert!(!world.spring_attached());

    world.spawn_attached_at(Vec2::new(320.0, 220.0));

    assert!(world.ball_visible());
    assert!(world.spring_attached());
    assert_relative_eq!(world.ball.pos.x, 320.0, epsilon = 0.01);
    assert_relative_eq!(world.ball.pos.y, 220.0, epsilon = 0.01);
    assert_relative_eq!(world.ball.vel.length(), 0.0, epsilon = 0.01);
}

#[test]
fn slow_cursor_sweep_does_not_entangle_spring() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.interact_spring(spring_mid + Vec2::new(-30.0, 0.0), 0.0);
    world.interact_spring(spring_mid + Vec2::new(30.0, 0.0), 0.25);

    assert!(world.spring.entanglement.is_none());
    assert!(
        world.spring.intersection.is_some(),
        "slow cursor pass should still displace the spring"
    );
}

#[test]
fn cursor_intersection_moves_ball_without_entangling() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    world.interact_spring(spring_mid + Vec2::new(-55.0, 0.0), 0.0);
    world.interact_spring(spring_mid + Vec2::new(55.0, 0.0), 0.22);
    assert!(world.spring.intersection.is_some());
    assert!(world.spring.entanglement.is_none());

    let x0 = world.ball.pos.x;
    for _ in 0..24 {
        world.advance(FIXED_DT);
    }

    assert!(
        (world.ball.pos.x - x0).abs() > 1.0,
        "spring deflection should tug the ball sideways, x0={x0}, pos={:?}",
        world.ball.pos
    );
}

#[test]
fn slow_right_click_hold_deflects_without_entangling() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    for i in 0..8 {
        let t = i as f32 / 7.0;
        let cursor = spring_mid + Vec2::new(-35.0 + 70.0 * t, 18.0);
        world.interact_spring(cursor, i as f32 * 0.08);
    }

    assert!(world.spring.intersection.is_some());
    assert!(world.spring.entanglement.is_none());
}

#[test]
fn right_click_side_support_persists_while_held() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    let cursor = spring_mid + Vec2::new(-72.0, 18.0);

    world.interact_spring(cursor, 0.0);

    let support = world
        .spring
        .intersection
        .expect("right-click near the spring should create a support point");
    assert!(world.spring.entanglement.is_none());
    assert!(support.point.x < spring_mid.x - 50.0);
    assert_relative_eq!(support.point.x, cursor.x, epsilon = 0.01);
    assert_relative_eq!(support.point.y, cursor.y, epsilon = 0.01);

    for _ in 0..120 {
        world.advance(FIXED_DT);
    }

    let held_support = world
        .spring
        .intersection
        .expect("held right-click support should not fade out while held");
    assert_relative_eq!(held_support.point.x, support.point.x, epsilon = 0.01);
    assert_relative_eq!(held_support.point.y, support.point.y, epsilon = 0.01);
    assert!(held_support.strength() > 0.99);
    assert!(world.spring.entanglement.is_none());
}

#[test]
fn right_click_support_follows_cursor_across_screen() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    let start_x = world.ball.pos.x;

    world.interact_spring(spring_mid + Vec2::new(-60.0, 0.0), 0.0);
    assert!(world.spring.intersection.is_some());

    let far_cursor = Vec2::new(world.bounds.right - 30.0, world.ball.pos.y);
    world.interact_spring(far_cursor, 0.2);

    let support = world
        .spring
        .intersection
        .expect("latched right-click support should follow the cursor across the screen");
    assert_relative_eq!(support.point.x, far_cursor.x, epsilon = 0.01);
    assert_relative_eq!(support.point.y, far_cursor.y, epsilon = 0.01);
    assert!(world.spring.entanglement.is_none());

    for _ in 0..80 {
        world.advance(FIXED_DT);
    }

    assert!(
        world.ball.pos.x > start_x + 80.0,
        "far cursor support should pull the ball across the screen, start_x={start_x}, pos={:?}",
        world.ball.pos
    );
    assert!(world.spring.entanglement.is_none());
}

#[test]
fn released_right_click_support_persists_until_momentum_drops() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    let cursor = spring_mid + Vec2::new(180.0, 80.0);

    world.interact_spring(spring_mid + Vec2::new(-60.0, 0.0), 0.0);
    world.interact_spring(cursor, 0.15);
    let held_support = world
        .spring
        .intersection
        .expect("right-click should latch a spring support");
    world.ball.vel = Vec2::new(700.0, 120.0);
    world.ball.spin = 12.0;

    world.stop_spring_interaction();

    let released_support = world
        .spring
        .intersection
        .expect("release should keep a moving support pinned temporarily");
    assert!(released_support.max_age.is_finite());
    assert!(released_support.cursor_vel.length() > 100.0);
    assert_relative_eq!(
        released_support.point.x,
        held_support.point.x,
        epsilon = 0.01
    );
    assert_relative_eq!(
        released_support.point.y,
        held_support.point.y,
        epsilon = 0.01
    );

    for _ in 0..20 {
        world.advance(FIXED_DT);
    }
    let still_supported = world
        .spring
        .intersection
        .expect("released support should survive while momentum remains");
    assert!(
        still_supported.point.distance(held_support.point) > 20.0,
        "released support should keep moving with inertia, held={:?}, supported={:?}",
        held_support.point,
        still_supported.point
    );

    world.spring.stiffness = 0.0;
    world.spring.damping = 0.0;
    world.ball.vel = Vec2::ZERO;
    world.ball.spin = 0.0;
    if let Some(intersection) = world.spring.intersection.as_mut() {
        intersection.cursor_vel = Vec2::ZERO;
    }
    for _ in 0..40 {
        world.advance(FIXED_DT);
    }

    assert!(
        world.spring.intersection.is_none(),
        "released support should clear after speed and spin fall away"
    );
}

#[test]
fn released_right_click_support_falls_with_gravity() {
    let mut world = World::new(
        WorldConfig::default(),
        Bounds::new(0.0, 0.0, 1000.0, 1200.0),
    );
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    world.interact_spring(spring_mid + Vec2::new(60.0, 0.0), 0.0);
    if let Some(intersection) = world.spring.intersection.as_mut() {
        intersection.cursor_vel = Vec2::ZERO;
    }
    let held_support = world
        .spring
        .intersection
        .expect("right-click should latch a spring support");
    world.ball.vel = Vec2::new(360.0, 0.0);

    world.stop_spring_interaction();

    for _ in 0..30 {
        world.advance(FIXED_DT);
    }

    let fallen_support = world
        .spring
        .intersection
        .expect("released support should remain while gravity acts on it");
    assert!(
        fallen_support.point.y > held_support.point.y + 8.0,
        "released support should fall under gravity, held={:?}, fallen={:?}",
        held_support.point,
        fallen_support.point
    );
    assert!(
        fallen_support.cursor_vel.y > 80.0,
        "gravity should add downward velocity to the support, vel={:?}",
        fallen_support.cursor_vel
    );
}

#[test]
fn cutting_displaced_spring_kicks_ball() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    world.interact_spring(spring_mid + Vec2::new(-80.0, 0.0), 0.0);
    world.interact_spring(spring_mid + Vec2::new(80.0, 0.0), 0.32);
    assert!(world.spring.intersection.is_some());
    assert!(world.spring.entanglement.is_none());

    world.cut_spring();

    assert!(!world.spring_attached());
    assert!(
        world.ball.vel.x.abs() > 50.0,
        "cutting a displaced spring should transfer cursor/string impulse, vel={:?}",
        world.ball.vel
    );
}

#[test]
fn fast_cursor_sweep_entangles_spring() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 10_000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.interact_spring(spring_mid + Vec2::new(-180.0, 0.0), 0.0);
    world.interact_spring(spring_mid + Vec2::new(180.0, 0.0), 0.06);

    assert!(
        world.spring.entanglement.is_some(),
        "fast cursor inertia near the spring should snag it"
    );
}

#[test]
fn entanglement_pushes_ball_around_cursor() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 10_000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    world.interact_spring(spring_mid + Vec2::new(-180.0, 0.0), 0.0);
    world.interact_spring(spring_mid + Vec2::new(180.0, 0.0), 0.06);

    let x0 = world.ball.pos.x;
    for _ in 0..30 {
        world.advance(FIXED_DT);
    }

    assert!(world.spring.entanglement.is_some());
    assert!(
        (world.ball.pos.x - x0).abs() > 6.0,
        "entangled ball should orbit laterally, x0={x0}, pos={:?}",
        world.ball.pos
    );
}

#[test]
fn cursor_entanglement_expires() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 10_000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    world.interact_spring(spring_mid + Vec2::new(-180.0, 0.0), 0.0);
    world.interact_spring(spring_mid + Vec2::new(180.0, 0.0), 0.06);
    assert!(world.spring.entanglement.is_some());

    for _ in 0..360 {
        world.advance(FIXED_DT);
    }

    assert!(world.spring.entanglement.is_none());
    assert!(world.spring_attached());
}

#[test]
fn passive_cursor_sweep_does_not_affect_spring() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.move_cursor(spring_mid + Vec2::new(-140.0, 0.0), 0.0);
    world.move_cursor(spring_mid + Vec2::new(140.0, 0.0), 0.04);

    assert!(world.spring.intersection.is_none());
    assert!(world.spring.entanglement.is_none());
}

#[test]
fn passive_cursor_sweep_does_not_bat_detached_ball() {
    let mut world = no_gravity_world();
    world.cut_spring();
    let center = world.ball.pos;

    world.move_cursor(center + Vec2::new(-220.0, 0.0), 0.0);
    world.move_cursor(center + Vec2::new(40.0, 0.0), 0.05);

    assert_relative_eq!(world.ball.vel.length(), 0.0, epsilon = 0.01);
}

#[test]
fn right_click_cursor_bats_detached_ball() {
    let mut world = no_gravity_world();
    world.cut_spring();
    let center = world.ball.pos;

    world.interact_spring(center + Vec2::new(-220.0, 0.0), 0.0);
    world.interact_spring(center + Vec2::new(40.0, 0.0), 0.05);

    assert!(
        world.ball.vel.x > 1000.0,
        "right-click cursor hit should transfer x momentum, vel={:?}",
        world.ball.vel
    );
    assert!(world.ball.vel.y.abs() < 50.0, "vel={:?}", world.ball.vel);
    assert!(!world.spring_attached());
}

#[test]
fn detached_ball_bounces_off_stationary_right_click_cursor() {
    let mut world = no_gravity_world();
    world.cut_spring();
    let cursor = world.ball.pos;
    world.ball.pos = cursor + Vec2::new(-100.0, 0.0);
    world.ball.vel = Vec2::new(1200.0, 0.0);

    world.interact_spring(cursor, 0.0);
    for _ in 0..12 {
        world.advance(FIXED_DT);
        if world.ball.vel.x < 0.0 {
            break;
        }
    }

    assert!(
        world.ball.vel.x < -500.0,
        "stationary right-click cursor should bounce the loose ball, vel={:?}",
        world.ball.vel
    );
}

#[test]
fn detached_ball_does_not_bounce_after_right_click_release() {
    let mut world = no_gravity_world();
    world.cut_spring();
    let cursor = world.ball.pos;
    world.ball.pos = cursor + Vec2::new(-100.0, 0.0);
    world.ball.vel = Vec2::new(1200.0, 0.0);

    world.interact_spring(cursor, 0.0);
    world.stop_spring_interaction();
    for _ in 0..12 {
        world.advance(FIXED_DT);
    }

    assert!(
        world.ball.vel.x > 900.0,
        "released right-click cursor should not keep bouncing the loose ball, vel={:?}",
        world.ball.vel
    );
}

#[test]
fn glancing_right_click_hit_spins_detached_ball() {
    let mut world = no_gravity_world();
    world.cut_spring();
    let center = world.ball.pos;
    let sweep_y = -world.ball.radius * 0.75;

    world.interact_spring(center + Vec2::new(-220.0, sweep_y), 0.0);
    world.interact_spring(center + Vec2::new(40.0, sweep_y), 0.05);

    assert!(
        world.ball.vel.x > 500.0,
        "glancing hit should carry cursor momentum, vel={:?}",
        world.ball.vel
    );
    assert!(
        world.ball.spin.abs() > 5.0,
        "glancing hit should spin the loose ball, spin={}",
        world.ball.spin
    );
}

#[test]
fn stopping_spring_interaction_clears_cursor_effects() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 10_000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.interact_spring(spring_mid + Vec2::new(-180.0, 0.0), 0.0);
    world.interact_spring(spring_mid + Vec2::new(180.0, 0.0), 0.06);
    assert!(world.spring.entanglement.is_some());

    world.stop_spring_interaction();

    assert!(world.spring.intersection.is_none());
    assert!(world.spring.entanglement.is_none());
}

#[test]
fn very_fast_cursor_sweep_cuts_spring() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 3000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.move_cursor(spring_mid + Vec2::new(-180.0, 0.0), 0.0);
    world.move_cursor(spring_mid + Vec2::new(180.0, 0.0), 0.04);

    assert!(!world.spring_attached());
}

#[test]
fn very_fast_parallel_cursor_sweep_does_not_cut_spring() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 3000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.move_cursor(spring_mid + Vec2::new(0.0, -180.0), 0.0);
    world.move_cursor(spring_mid + Vec2::new(0.0, 180.0), 0.04);

    assert!(
        world.spring_attached(),
        "fast tangent motion along the spring should not cut it"
    );
}

#[test]
fn very_fast_cursor_sweep_through_ball_does_not_cut_spring() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 3000.0;
    let ball = world.ball.pos;

    world.move_cursor(ball + Vec2::new(-180.0, 0.0), 0.0);
    world.move_cursor(ball + Vec2::new(180.0, 0.0), 0.04);

    assert!(
        world.spring_attached(),
        "swiping through the ball should not cut the protected ball-side band"
    );
}

#[test]
fn very_fast_cursor_sweep_near_ball_end_does_not_cut_spring() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 3000.0;
    let near_ball = world.ball.pos + (world.spring.anchor - world.ball.pos).normalize() * 64.0;

    world.move_cursor(near_ball + Vec2::new(-180.0, 0.0), 0.0);
    world.move_cursor(near_ball + Vec2::new(180.0, 0.0), 0.04);

    assert!(
        world.spring_attached(),
        "the band segment near the ball should be uncuttable"
    );
}

#[test]
fn very_fast_right_click_sweep_does_not_cut_spring() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 3000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.interact_spring(spring_mid + Vec2::new(-180.0, 0.0), 0.0);
    world.interact_spring(spring_mid + Vec2::new(180.0, 0.0), 0.04);

    assert!(world.spring_attached());
}

#[test]
fn slow_cursor_sweep_does_not_cut_spring() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 3000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.move_cursor(spring_mid + Vec2::new(-120.0, 0.0), 0.0);
    world.move_cursor(spring_mid + Vec2::new(120.0, 0.0), 0.5);

    assert!(world.spring_attached());
}

#[test]
fn stationary_cursor_cuts_fast_moving_spring() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 3000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.move_cursor(spring_mid, 0.0);
    world.ball.vel = Vec2::new(3600.0, 0.0);
    world.advance(FIXED_DT);

    assert!(
        !world.spring_attached(),
        "fast ball motion relative to a stationary cursor should cut the spring"
    );
}

#[test]
fn stationary_cursor_does_not_cut_slow_moving_spring() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 3000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.move_cursor(spring_mid, 0.0);
    world.ball.vel = Vec2::new(1200.0, 0.0);
    world.advance(FIXED_DT);

    assert!(world.spring_attached());
}

#[test]
fn stationary_cursor_does_not_cut_fast_tangent_spring_motion() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 3000.0;
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.move_cursor(spring_mid, 0.0);
    world.ball.vel = Vec2::new(0.0, 3600.0);
    world.advance(FIXED_DT);

    assert!(
        world.spring_attached(),
        "fast motion along the spring tangent should not cut it"
    );
}

#[test]
fn stationary_cursor_does_not_cut_fast_ball_end_motion() {
    let mut world = no_gravity_world();
    world.config.cut_spring_cursor_speed = 3000.0;
    world.move_cursor(world.ball.pos, 0.0);
    world.ball.vel = Vec2::new(3600.0, 0.0);
    world.advance(FIXED_DT);

    assert!(
        world.spring_attached(),
        "stationary cursor inside the ball should not cut the protected ball-side band"
    );
}

#[test]
fn hit_test_inside_and_outside() {
    let ball = Ball::new(Vec2::new(100.0, 100.0), 40.0);
    assert!(InteractionState::hit_test(&ball, Vec2::new(110.0, 110.0)));
    assert!(!InteractionState::hit_test(&ball, Vec2::new(200.0, 200.0)));
}

#[test]
fn grab_only_when_over_ball() {
    let mut world = no_gravity_world();
    let center = world.bounds.center();
    assert!(world.grab(center, 0.0));
    assert!(world.is_grabbed());
    world.release(0.1);
    assert!(!world.is_grabbed());

    assert!(!world.grab(center + Vec2::new(500.0, 0.0), 0.2));
}

#[test]
fn throw_imparts_velocity_in_drag_direction() {
    let mut world = no_gravity_world();
    let start = world.bounds.center();
    let mut now = 0.0;
    assert!(world.grab(start, now));
    let vel = Vec2::new(800.0, -400.0);
    for _ in 0..8 {
        now += 0.01;
        world.move_cursor(start + vel * now, now);
        world.advance(0.01);
    }
    world.release(now);
    // Thrown roughly along the drag direction.
    assert!(world.ball.vel.x > 100.0, "vx={}", world.ball.vel.x);
    assert!(world.ball.vel.y < -50.0, "vy={}", world.ball.vel.y);
}

#[test]
fn off_center_fast_drag_builds_spin() {
    let mut world = no_gravity_world();
    world.cut_spring();
    let start = world.ball.pos;
    let grab = start + Vec2::new(0.0, -world.ball.radius * 0.6);
    let drag_vel = Vec2::new(1800.0, 0.0);
    let mut now = 0.0;

    assert!(world.grab(grab, now));
    for _ in 0..12 {
        now += 0.01;
        world.move_cursor(grab + drag_vel * now, now);
        world.advance(0.01);
    }

    assert!(
        world.ball.spin > 2.0,
        "off-center fast drag should spin the ball, spin={}",
        world.ball.spin
    );
}

#[test]
fn spin_curves_released_ball() {
    let cfg = WorldConfig {
        gravity: Vec2::ZERO,
        ..WorldConfig::default()
    };
    let mut world = World::new(cfg, Bounds::new(0.0, 0.0, 4000.0, 1200.0));
    world.cut_spring();
    let start = world.ball.pos;
    let grab = start + Vec2::new(0.0, -world.ball.radius * 0.7);
    let drag_vel = Vec2::new(2200.0, 0.0);
    let mut now = 0.0;

    assert!(world.grab(grab, now));
    for _ in 0..14 {
        now += 0.01;
        world.move_cursor(grab + drag_vel * now, now);
        world.advance(0.01);
    }
    world.release(now);

    let release_y = world.ball.pos.y;
    assert!(world.ball.spin > 4.0, "spin={}", world.ball.spin);
    assert!(
        world.ball.vel.y.abs() < 1.0,
        "release velocity should start straight enough to isolate spin, vel={:?}",
        world.ball.vel
    );

    for _ in 0..80 {
        world.advance(FIXED_DT);
    }

    assert!(
        world.ball.pos.y > release_y + 18.0,
        "positive spin should curve the rightward throw downward, y0={release_y}, pos={:?}",
        world.ball.pos
    );
}

#[test]
fn spin_falls_off_and_allows_sleep() {
    let mut world = no_gravity_world();
    world.cut_spring();
    world.ball.spin = 24.0;
    let initial_spin = world.ball.spin;

    for _ in 0..120 {
        world.advance(FIXED_DT);
    }
    assert!(
        world.ball.spin.abs() < initial_spin,
        "spin should be damped by friction, spin={}",
        world.ball.spin
    );

    for _ in 0..1100 {
        world.advance(FIXED_DT);
    }
    assert_relative_eq!(world.ball.spin, 0.0, epsilon = 0.001);
    assert!(world.ball.asleep, "ball should sleep after spin fades");
}

#[test]
fn velocity_is_clamped_to_max_speed() {
    let mut world = no_gravity_world();
    world.ball.vel = Vec2::new(99_999.0, 99_999.0);
    world.advance(FIXED_DT);
    assert!(world.ball.speed() <= world.config.max_speed + 1.0);
}

#[test]
fn particles_emit_and_decay() {
    let mut ps = ParticleSystem::new(256);
    ps.emit_burst(Vec2::ZERO, 1000.0, Vec4::ONE);
    assert!(!ps.is_empty());
    let n0 = ps.len();
    for _ in 0..200 {
        ps.update(0.02, 1.0);
    }
    assert!(ps.len() < n0, "particles should expire over time");
}

#[test]
fn particle_pool_respects_capacity() {
    let mut ps = ParticleSystem::new(32);
    for _ in 0..10 {
        ps.emit_burst(Vec2::ZERO, 4000.0, Vec4::ONE);
    }
    assert!(ps.len() <= 32);
}

#[test]
fn trail_records_and_fades() {
    let mut trail = Trail::new(32, 0.5);
    for i in 0..20 {
        trail.record(Vec2::new(i as f32 * 10.0, 0.0), 40.0);
    }
    assert!(trail.len() > 1);
    for _ in 0..40 {
        trail.update(0.05);
    }
    assert!(trail.is_empty(), "trail should fully fade");
}

#[test]
fn nudge_launches_ball_at_speed() {
    let mut world = no_gravity_world();
    world.nudge(2800.0);
    assert_relative_eq!(world.ball.speed(), 2800.0, epsilon = 1.0);
    assert!(!world.ball.asleep);
}

#[test]
fn ball_rolls_by_distance_travelled() {
    let mut ball = Ball::new(Vec2::ZERO, 10.0);

    ball.roll_by(Vec2::new(10.0, 0.0));
    assert_relative_eq!(ball.roll_angle, 1.0, epsilon = 0.001);
    assert_relative_eq!(ball.roll_dir.x, 1.0, epsilon = 0.001);
    assert_relative_eq!(ball.roll_dir.y, 0.0, epsilon = 0.001);

    ball.roll_by(Vec2::new(0.0, 5.0));
    assert_relative_eq!(ball.roll_angle, 1.5, epsilon = 0.001);
    assert_relative_eq!(ball.roll_dir.x, 0.0, epsilon = 0.001);
    assert_relative_eq!(ball.roll_dir.y, 1.0, epsilon = 0.001);
}

#[test]
fn world_advances_ball_roll_with_motion() {
    let mut world = no_gravity_world();
    world.ball.vel = Vec2::new(600.0, 0.0);

    world.advance(FIXED_DT);

    assert!(world.ball.roll_angle > 0.0);
    assert_relative_eq!(world.ball.roll_dir.x, 1.0, epsilon = 0.001);
    assert_relative_eq!(world.ball.roll_dir.y, 0.0, epsilon = 0.001);
}

#[test]
fn ball_sleeps_when_still_without_gravity() {
    let mut world = no_gravity_world();
    world.ball.vel = Vec2::ZERO;
    for _ in 0..400 {
        world.advance(FIXED_DT);
    }
    assert!(world.ball.asleep, "ball should sleep after being still");
    assert!(!world.is_active());
}
