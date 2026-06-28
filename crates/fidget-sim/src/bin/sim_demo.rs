//! Headless demonstration of the fidget simulation.
//!
//! Grabs the ball, flicks it across the screen, releases, and reports the
//! trajectory + bounce count. Useful for verifying the physics without a GPU.
//!
//! Run with:
//!   cargo run -p fidget-sim --bin sim_demo --target x86_64-unknown-linux-gnu

use fidget_sim::{Bounds, World, WorldConfig, FIXED_DT};
use glam::Vec2;

fn main() {
    let bounds = Bounds::new(0.0, 0.0, 1280.0, 720.0);
    // Turn gravity off for a deterministic, easy-to-read bounce demo.
    let cfg = WorldConfig {
        gravity: Vec2::ZERO,
        ..WorldConfig::default()
    };
    let mut world = World::new(cfg, bounds);

    println!("Fidget-VK headless simulation demo");
    println!("bounds: {}x{}  ball radius: {}", bounds.width(), bounds.height(), world.ball.radius);

    // Simulate a drag-then-throw gesture from the centre to the right+down.
    let mut now = 0.0_f32;
    let start = world.bounds.center();
    assert!(world.grab(start, now), "expected to grab ball at centre");
    println!("\ngrabbed ball at {:?}", world.ball.pos);

    // Move the cursor quickly over ~80ms to build up throw velocity.
    let flick = Vec2::new(900.0, 500.0); // px/s
    let steps = 8;
    for _ in 0..steps {
        now += 0.01;
        let cursor = start + flick * now;
        world.move_cursor(cursor, now);
        world.advance(0.01);
    }
    world.release(now);
    println!(
        "released: pos={:?} vel={:?} (speed {:.0} px/s)\n",
        world.ball.pos,
        world.ball.vel,
        world.ball.speed()
    );

    // Free flight: advance ~3 seconds and watch it bounce off the walls.
    let mut bounces = 0;
    let mut last_vx = world.ball.vel.x.signum();
    let mut last_vy = world.ball.vel.y.signum();
    let total_steps = (3.0 / FIXED_DT) as usize;
    for i in 0..total_steps {
        world.advance(FIXED_DT);
        now += FIXED_DT;

        let vx = world.ball.vel.x.signum();
        let vy = world.ball.vel.y.signum();
        if vx != last_vx || vy != last_vy {
            bounces += 1;
            println!(
                "  bounce #{:<2} at t={:>5.2}s  pos=({:>6.1}, {:>6.1})  speed={:>5.0}",
                bounces,
                now,
                world.ball.pos.x,
                world.ball.pos.y,
                world.ball.speed()
            );
        }
        last_vx = vx;
        last_vy = vy;

        if i % 40 == 0 {
            // Periodic sample so the trajectory is visible in the output.
            println!(
                "  t={:>5.2}s pos=({:>6.1}, {:>6.1}) speed={:>5.0} particles={} trail={}",
                now,
                world.ball.pos.x,
                world.ball.pos.y,
                world.ball.speed(),
                world.particles.len(),
                world.trail.len()
            );
        }
    }

    // Sanity: the ball must always remain inside the bounds.
    let r = world.ball.radius;
    let inside = world.ball.pos.x >= bounds.left + r - 0.5
        && world.ball.pos.x <= bounds.right - r + 0.5
        && world.ball.pos.y >= bounds.top + r - 0.5
        && world.ball.pos.y <= bounds.bottom - r + 0.5;

    println!("\nfinal pos=({:.1}, {:.1})  bounces={}  inside_bounds={}", world.ball.pos.x, world.ball.pos.y, bounces, inside);
    assert!(inside, "ball escaped its bounds!");
    assert!(bounces >= 1, "expected at least one wall bounce");
    println!("OK: simulation behaved correctly ({} bounces, stayed in bounds)", bounces);
}
