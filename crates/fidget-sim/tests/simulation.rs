use approx::assert_relative_eq;
use fidget_sim::{Ball, Bounds, InteractionState, ParticleSystem, Trail, World, WorldConfig, FIXED_DT};
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
    assert!(bounced, "ball should reverse x velocity after hitting right wall");
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
    let mut world = World::new(WorldConfig::default(), Bounds::new(0.0, 0.0, 1000.0, 6000.0));
    world.ball.pos = Vec2::new(500.0, 100.0);
    world.ball.vel = Vec2::ZERO;
    let y0 = world.ball.pos.y;
    for _ in 0..30 {
        world.advance(FIXED_DT);
    }
    assert!(world.ball.pos.y > y0, "gravity should increase y (downward)");
    assert!(world.ball.vel.y > 0.0);
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
fn ball_sleeps_when_still_without_gravity() {
    let mut world = no_gravity_world();
    world.ball.vel = Vec2::ZERO;
    for _ in 0..400 {
        world.advance(FIXED_DT);
    }
    assert!(world.ball.asleep, "ball should sleep after being still");
    assert!(!world.is_active());
}
