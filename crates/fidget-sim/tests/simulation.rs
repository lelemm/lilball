use approx::assert_relative_eq;
use fidget_sim::{
    Ball, Bounds, InteractionState, ParticleSystem, Trail, World, WorldConfig, FIXED_DT,
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
fn fallen_ball_recalls_to_spring() {
    let mut world = World::new(WorldConfig::default(), Bounds::new(0.0, 0.0, 1000.0, 600.0));
    world.cut_spring();
    world.ball.pos = Vec2::new(
        500.0,
        world.bounds.bottom + world.spring.recall_margin + world.ball.radius + 2.0,
    );
    world.ball.vel = Vec2::new(0.0, 1200.0);

    world.advance(FIXED_DT);

    assert!(world.spring_attached());
    assert_relative_eq!(
        world.ball.pos.x,
        world.spring.rest_position().x,
        epsilon = 0.01
    );
    assert_relative_eq!(
        world.ball.pos.y,
        world.spring.rest_position().y,
        epsilon = 0.01
    );
    assert_relative_eq!(world.ball.vel.length(), 0.0, epsilon = 0.01);
}

#[test]
fn slow_cursor_sweep_does_not_entangle_spring() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.move_cursor(spring_mid + Vec2::new(-30.0, 0.0), 0.0);
    world.move_cursor(spring_mid + Vec2::new(30.0, 0.0), 0.25);

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
    world.move_cursor(spring_mid + Vec2::new(-55.0, 0.0), 0.0);
    world.move_cursor(spring_mid + Vec2::new(55.0, 0.0), 0.22);
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
fn cutting_displaced_spring_kicks_ball() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    world.move_cursor(spring_mid + Vec2::new(-80.0, 0.0), 0.0);
    world.move_cursor(spring_mid + Vec2::new(80.0, 0.0), 0.32);
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
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;

    world.move_cursor(spring_mid + Vec2::new(-140.0, 0.0), 0.0);
    world.move_cursor(spring_mid + Vec2::new(140.0, 0.0), 0.04);

    assert!(
        world.spring.entanglement.is_some(),
        "fast cursor inertia near the spring should snag it"
    );
}

#[test]
fn entanglement_pushes_ball_around_cursor() {
    let mut world = no_gravity_world();
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    world.move_cursor(spring_mid + Vec2::new(-140.0, 0.0), 0.0);
    world.move_cursor(spring_mid + Vec2::new(140.0, 0.0), 0.04);

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
    let spring_mid = (world.spring.anchor + world.ball.pos) * 0.5;
    world.move_cursor(spring_mid + Vec2::new(-140.0, 0.0), 0.0);
    world.move_cursor(spring_mid + Vec2::new(140.0, 0.0), 0.04);
    assert!(world.spring.entanglement.is_some());

    for _ in 0..360 {
        world.advance(FIXED_DT);
    }

    assert!(world.spring.entanglement.is_none());
    assert!(world.spring_attached());
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
