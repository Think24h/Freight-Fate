//! A co-driver whose hours run out before the player's rest is in, with the
//! player out of hours of their own, ends the shift at the last lot their
//! hours reach instead of handing over on the highway (player report,
//! 2026-10-20: Lou handed back at 55 miles per hour with the duty window
//! closed, and a trooper's log check followed).

use ff_core::sim::weather::WeatherKind;
use freight_fate::playtest::harness::{PlaytestHarness, StartDelivery};
use freight_fate::states::base::Key;
use freight_fate::states::driving_team::TeamMode;

const DT: f64 = 1.0 / 60.0;

fn quiet_drive(name: &str) -> PlaytestHarness {
    let mut harness = PlaytestHarness::new();
    harness.start_delivery(StartDelivery::named(name));
    harness.app.ctx.settings.hos_mode = "realistic".to_string();
    harness.with_drive(|d, _| {
        d.tutorial = None;
        d.departure_checked = true;
        d.trip.hazard_check_mi = 1e9;
        d.trip.inspection_check_mi = 1e9;
        d.trip.traffic_manager.rolling_bubble = false;
        d.trip.set_npc_vehicles(Vec::new());
        d.trip.traffic_pressures.clear();
        d.trip.zones.retain(|z| z.aadt.is_none());
        d.trip.weather.current = WeatherKind::Clear;
        d.trip.set_patrols(Vec::new());
        d.trip.posts.clear();
    });
    harness
}

fn tail(harness: &PlaytestHarness) -> String {
    let lines = harness.transcript();
    let from = lines.len().saturating_sub(30);
    lines[from..].join("\n")
}

/// The player has driven `player_min`, hands a sleeping shift to Dan, and
/// Dan has an hour of driving left. Two lots ahead: one 15 miles on,
/// inside his hours, and one 50 miles on, past them.
fn short_shift(name: &str, player_min: f64) -> PlaytestHarness {
    short_shift_in(name, player_min, TeamMode::SleepThrough)
}

fn short_shift_in(name: &str, player_min: f64, mode: TeamMode) -> PlaytestHarness {
    let mut harness = quiet_drive(name);
    let hos = &mut harness.app.ctx.profile.as_mut().expect("a career").hos;
    hos.drive(player_min);
    harness.with_drive(|d, ctx| {
        d.truck_mut().start_engine();
        d.truck_mut().set_air_ready(false);
        let lot = d
            .trip
            .stops
            .iter()
            .find(|stop| {
                stop.parking != "none"
                    && stop.stop_type != "weigh_station"
                    && stop.stop_type != "delivery_destination"
            })
            .cloned()
            .expect("a stop with parking");
        let here = d.trip.position_mi;
        d.trip
            .stops
            .retain(|stop| stop.stop_type == "delivery_destination");
        for (ahead, tag) in [(15.0, "near"), (50.0, "far")] {
            let mut stop = lot.clone();
            stop.at_mi = here + ahead;
            stop.name = format!("{} {tag}", stop.name);
            d.trip.stops.push(stop);
        }
        d.trip.stops.sort_by(|a, b| a.at_mi.total_cmp(&b.at_mi));
        d.truck_mut().velocity_mps = 60.0 / 2.236_936;
        d.cruise_mph = Some(60.0);
        d.hand_wheel_to_co_driver(ctx, mode);
        d.truck_mut().parking_brake = false;
    });
    let hos = &mut harness.app.ctx.profile.as_mut().expect("a career").hos;
    hos.drive(7.0 * 60.0);
    hos.take_break(30.0);
    hos.drive(3.0 * 60.0);
    harness
}

#[test]
fn out_of_hours_the_player_is_handed_the_wheel_parked_at_the_last_lot() {
    let mut harness = short_shift("Team Last Lot", 14.0 * 60.0);
    let last = harness.read_drive(|d| {
        d.co_driver_last_lot(&harness.app.ctx)
            .map(|stop| stop.name.clone())
    });
    assert!(
        last.as_deref().is_some_and(|name| name.ends_with(" near")),
        "{last:?}"
    );
    let jump = harness.with_drive(|d, ctx| d.sleep_ahead(ctx));
    harness.app.ctx.run_deferred();
    assert!(jump.is_some(), "the shift passes as time");
    for _ in 0..60 * 60 {
        if harness.read_drive(|d| d.team.as_ref().is_some_and(|team| team.wheel.is_none())) {
            break;
        }
        harness.advance_frame_clock();
        harness.app.frame(DT);
    }
    let said = harness.transcript().join("\n");
    assert!(
        said.contains(
            "Dan has parked for the end of their shift. Their hours run out before the next \
             place to park."
        ),
        "{}",
        tail(&harness)
    );
    assert!(said.contains("of your 10-hour rest still to go."), "{said}");
    assert!(!said.contains("Their hours are up"), "{said}");
    let speed = harness.read_drive(|d| d.trip.truck.speed_mph());
    assert!(speed < 1.0, "handed back parked, not on the road: {speed}");
}

#[test]
fn riding_along_out_of_hours_the_co_driver_pulls_in_at_the_last_lot() {
    let mut harness = short_shift_in("Team Last Lot Ride", 14.0 * 60.0, TeamMode::RideAlong);
    let mut back = false;
    for _ in 0..60 * 60 * 20 {
        harness.advance_frame_clock();
        harness.app.frame(DT);
        harness.app.ctx.run_deferred();
        if !harness.app.ctx.co_driver_away {
            back = true;
            break;
        }
    }
    assert!(back, "{}", tail(&harness));
    let said = harness.transcript().join("\n");
    assert!(
        said.contains("Dan has parked for the end of their shift."),
        "{}",
        tail(&harness)
    );
    assert!(!said.contains("Their hours are up"), "{said}");
}

#[test]
fn with_hours_of_their_own_the_player_takes_over_on_the_road_as_before() {
    let harness = short_shift("Team Last Lot Hours", 300.0);
    let last = harness.read_drive(|d| d.co_driver_last_lot(&harness.app.ctx).is_some());
    assert!(!last, "the player can drive on to a lot");
}

#[test]
fn the_handover_help_says_when_the_co_driver_is_short_of_the_players_rest() {
    let mut harness = quiet_drive("Team Last Lot Help");
    let hos = &mut harness.app.ctx.profile.as_mut().expect("a career").hos;
    hos.drive(300.0);
    harness.with_drive(|d, ctx| d.hand_wheel_to_co_driver(ctx, TeamMode::RideAlong));
    let hos = &mut harness.app.ctx.profile.as_mut().expect("a career").hos;
    hos.drive(4.0 * 60.0);
    harness.with_drive(|d, ctx| {
        d.take_wheel_back(ctx);
    });
    harness.press_key(Key::Escape, None);
    let help = harness.with_state::<freight_fate::states::driving_pause_states::PauseMenuState, _>(
        |pause, ctx| {
            use freight_fate::states::base::Menu;
            let items = pause.build_items(ctx);
            items
                .iter()
                .find(|item| item.label.text(pause, ctx) == "Hand the wheel to Dan and sleep")
                .map(|item| item.help.text(pause, ctx))
        },
    );
    let help = help.expect("the handover row");
    assert!(
        help.contains("Dan has 6.8 hours of driving left, short of your 10-hour rest."),
        "{help}"
    );
}
