use crate::frame::FrameProfiler;
use crate::frame::Player;
use crate::heightmap_phy;
use crate::mobile;
use crate::utils;
use mobile::*;
use na::{Matrix4, Point2, Point3, Vector2, Vector3};
use std::collections::{HashMap, HashSet};
use utils::*;

const PHY_S: f32 = 0.100;
const PHY_MS: f32 = 100.0;
pub struct Group {}

impl Group {
    pub fn update_mobile_target(
        mouse_trigger: &HashSet<winit::event::MouseButton>,
        mouse_world_pos: Option<Vector3<f32>>,
        selected: &HashSet<IdValue>,
        kbots: &mut HashMap<Id<KBot>, KBot>,
    ) {
        match (
            mouse_trigger.contains(&winit::event::MouseButton::Right),
            mouse_world_pos,
        ) {
            (true, Some(mouse_pos_world)) => {
                let selected_count = selected.len();
                let formation_w = (selected_count as f32).sqrt().ceil() as i32;

                let mut spot = Vec::<Vector3<f32>>::new();
                for i in 0..formation_w {
                    for j in 0..formation_w {
                        spot.push(
                            mouse_pos_world
                                + Vector3::new(
                                    i as f32 - formation_w as f32 / 2.0,
                                    j as f32 - formation_w as f32 / 2.0,
                                    0.0,
                                ) * 4.0,
                        )
                    }
                }

                let mut center = Vector3::new(0.0, 0.0, 0.0);
                let mut tap = 0.0;

                let mut id_to_pos = Vec::new();
                for &s in selected.iter() {
                    if let Some(mobile) = kbots.get(&Id::new(s)) {
                        id_to_pos.push((mobile.id, mobile.position.coords));
                        center += mobile.position.coords;
                        tap += 1.0;
                    }
                }
                center /= tap;

                let axis = (mouse_pos_world - center).normalize();

                let mut projected_spot: Vec<_> = spot
                    .iter()
                    .enumerate()
                    .map(|(index, v)| (index, v.dot(&axis)))
                    .collect();

                projected_spot.sort_by(|(_, proj), (_, proj2)| {
                    if proj > proj2 {
                        std::cmp::Ordering::Greater
                    } else {
                        std::cmp::Ordering::Less
                    }
                });

                let mut id_to_proj: Vec<_> = id_to_pos
                    .iter()
                    .map(|(index, v)| (index, v.dot(&axis)))
                    .collect();

                id_to_proj.sort_by(|(_, proj), (_, proj2)| {
                    if proj > proj2 {
                        std::cmp::Ordering::Greater
                    } else {
                        std::cmp::Ordering::Less
                    }
                });

                for ((id, _), (spot_id, _)) in id_to_proj.iter().zip(&projected_spot[..]) {
                    if let Some(mobile) = kbots.get_mut(id) {
                        println!("New order for {}", mobile.id);
                        mobile.target = Some(Point3::<f32>::from(spot[*spot_id]));
                    }
                }
            }
            _ => {}
        }
    }

    pub fn update_units(
        frame_profiler: &mut FrameProfiler,
        kbots: &mut HashMap<Id<KBot>, KBot>,
        kinematic_projectiles: &mut HashMap<Id<KinematicProjectile>, KinematicProjectile>,
        heightmap_phy: &heightmap_phy::HeightmapPhy,
        arrows: &mut Vec<Arrow>,
        frame_count: i32,
        players: &HashMap<Id<Player>, Player>,
    ) {
        let cell_size = 16;
        let grid_w = (heightmap_phy.width / cell_size) as usize;
        let grid_h = (heightmap_phy.height / cell_size) as usize;

        let start = std::time::Instant::now();
        let mut grid = vec![Vec::<Id<KBot>>::new(); grid_w * grid_h];

        let grid_pos = |mobile: &KBot| -> usize {
            let (x, y) = (mobile.position.x, mobile.position.y);
            (x as usize / cell_size as usize) as usize
                + (y as usize / cell_size as usize) as usize * grid_w
        };

        for (&id, mobile) in kbots.iter() {
            let gp = grid_pos(mobile);
            grid[gp].push(id);

            for cell in &[
                -1_i32 - grid_w as i32,
                -(grid_w as i32),
                1 - grid_w as i32,
                -1,
                1,
                -1 + grid_w as i32,
                grid_w as i32,
                1 + grid_w as i32,
            ] {
                let cell_index = cell + gp as i32;
                if cell_index >= 0 && (cell_index as usize) < grid_w * grid_h {
                    grid[cell_index as usize].push(id);
                }
            }
        }

        frame_profiler.add("1  grid", start.elapsed());

        let mobiles2 = kbots.clone();
        //Movement compute

        for (id, mobile) in kbots.iter_mut() {
            let grid_pos = grid_pos(mobile);

            let mut neighbors_id: Vec<Id<KBot>> = grid[grid_pos].clone();
            let to_remove = neighbors_id.iter().position(|e| e == id).unwrap();
            neighbors_id.remove(to_remove);

            let mut collision_avoid_dir = Vector2::new(0.0_f32, 0.0);
            let mut collision_avoid_priority = 0.0_f32;

            let mut neighbor_dir = Vector2::new(0.0_f32, 0.0);
            let mut neighbor_dir_priority = 0.0_f32;

            if neighbors_id.len() == 0 {
            } else {
                let frame_prediction = 1.5;
                let mut nearest = None;
                let mut dist_min = None;
                for neighbor_id in neighbors_id.iter() {
                    let neighbor = mobiles2.get(neighbor_id).unwrap();
                    let dist = (neighbor.position + neighbor.speed * frame_prediction
                        - &mobile.position
                        - mobile.speed * frame_prediction)
                        .norm_squared();

                    let is_better = match dist_min {
                        None => true,
                        Some(dist_min) => dist_min > dist,
                    };

                    if is_better {
                        dist_min = Some(dist);
                        nearest = Some(neighbor);
                    }
                }

                let dist_min = dist_min.unwrap().sqrt();
                let nearest = nearest.unwrap();

                if dist_min < 4.0 {
                    let closeness = (mobile.position.coords + mobile.speed * frame_prediction
                        - nearest.position.coords
                        - nearest.speed * frame_prediction)
                        .xy()
                        .magnitude();

                    collision_avoid_priority = ((4.0 - closeness) / 4.0).max(0.0).min(0.8);

                    collision_avoid_dir = if nearest.speed.xy().norm_squared() < 0.01
                        || mobile.speed.xy().dot(&nearest.speed.xy()) < 0.0
                    {
                        let u = (mobile.position.coords - nearest.position.coords)
                            .xy()
                            .normalize();

                        let v = Vector2::<f32>::new(u.y, -u.x).normalize();
                        let w = Vector2::<f32>::new(-u.y, u.x).normalize();

                        if v.dot(&mobile.speed.xy()) > w.dot(&mobile.speed.xy()) {
                            v
                        } else {
                            w
                        }
                    } else {
                        let him_to_me = (mobile.position.coords - nearest.position.coords)
                            .xy()
                            .normalize();
                        him_to_me
                    };

                    // arrows.push(Arrow {
                    //     position: mobile.position,
                    //     color: [collision_avoid_priority, 0.0, 0.0, 0.0],
                    //     end: mobile.position
                    //         + Vector3::new(
                    //             collision_avoid_dir.x * 2.0,
                    //             collision_avoid_dir.y * 2.0,
                    //             0.0,
                    //         ),
                    // });

                    let speed_closeness = mobile.speed.xy().dot(&nearest.speed.xy());
                    neighbor_dir_priority = if speed_closeness > 0.0 && nearest.speed.norm() > 0.1 {
                        speed_closeness
                            .max(0.0)
                            .min((1.0 - collision_avoid_priority) * 0.2)
                    } else {
                        0.0
                    };

                    neighbor_dir = nearest
                        .speed
                        .xy()
                        .try_normalize(0.001)
                        .unwrap_or(Vector2::new(0.0, 0.0));

                    // arrows.push(Arrow {
                    //     position: mobile.position,
                    //     color: [0.0, neighbor_dir_priority, 0.0, 0.0],
                    //     end: mobile.position
                    //         + Vector3::new(neighbor_dir.x * 2.0, neighbor_dir.y * 2.0, 0.0),
                    // });
                }
            }

            let mut dir_intensity = 0.0;

            if let Some(target) = mobile.target {
                let to_target = (target.coords - mobile.position.coords).xy();
                let to_target_distance = to_target.norm();
                let will_to_go_target = if to_target_distance > 0.5 {
                    (to_target_distance / 2.0).min(1.0)
                } else {
                    0.0
                };

                let target_dir = to_target.normalize();
                let available = (1.0 - collision_avoid_priority - neighbor_dir_priority).max(0.0);
                let target_prio = available.min(will_to_go_target);
                let dir = target_dir * target_prio
                    + collision_avoid_dir * collision_avoid_priority
                    + neighbor_dir * neighbor_dir_priority;

                dir_intensity = target_prio + collision_avoid_priority + neighbor_dir_priority;
                mobile.dir = mobile.dir * 0.50 + Vector3::new(dir.x, dir.y, 0.0) * 0.5;
                if will_to_go_target < 0.01 {
                    mobile.target = None;
                }
            }
            mobile.speed = (mobile.speed + mobile.dir * 10.8 * dir_intensity) * 0.1;
            mobile.position += mobile.speed;
            mobile.position.z = heightmap_phy.z_linear(mobile.position.x, mobile.position.y) + 0.5;
        }

        //AABB for kbot and proj
        {
            let start = std::time::Instant::now();
            let cell_size = 2;
            let grid_w = (heightmap_phy.width / cell_size) as usize;
            let grid_h = (heightmap_phy.height / cell_size) as usize;
            let mut small_grid_kbot = vec![Vec::<Id<KBot>>::new(); grid_w * grid_h];

            fn insert_aabb<T>(
                position: Vector3<f32>,
                radius: f32,
                id: T,
                grid: &mut Vec<Vec<T>>,
                cell_size: usize,
                grid_w: usize,
            ) where
                T: Copy + std::fmt::Display,
            {
                let min_x = (position.x - radius * 2.0).floor() as usize;
                let max_x = (position.x + radius * 2.0).ceil() as usize;
                let min_y = (position.y - radius * 2.0).floor() as usize;
                let max_y = (position.y + radius * 2.0).ceil() as usize;

                let min_x = (min_x / cell_size);
                let max_x = ((max_x + 1) / cell_size);
                let min_y = (min_y / cell_size);
                let max_y = ((max_y + 1) / cell_size);

                for i in (min_x..=max_x).step_by(cell_size) {
                    for j in (min_y..=max_y).step_by(cell_size) {
                        // println!("INSERTION {} {} {}", i, j, id);
                        grid[i + j * grid_w].push(id);
                    }
                }
            }

            for (id, kbot) in kbots.iter() {
                insert_aabb::<Id<KBot>>(
                    kbot.position.coords,
                    kbot.radius,
                    *id,
                    &mut small_grid_kbot,
                    cell_size,
                    grid_w,
                );
            }

            frame_profiler.add("3  small_grid", start.elapsed());

            let start = std::time::Instant::now();
            //Projectile move compute
            {
                let mut to_remove = HashSet::new();
                for proj in kinematic_projectiles.values_mut() {
                    let current_pos = proj.positions.iter().next().unwrap();
                    let next_pos = proj.positions.iter().skip(1).next();

                    if let Some(next_pos) = next_pos {
                        //Slowly interpolate to not miss collisions
                        let step_size = proj.radius * 1.0;
                        let ul = next_pos.coords - current_pos.coords;
                        let distance_to_travel = ul.magnitude();
                        let u = ul / distance_to_travel;

                        let mut current_interp = current_pos.coords.clone();
                        let count = (distance_to_travel / step_size).floor() as usize;
                        for n in 0..=count {
                            current_interp += u * step_size;
                            if n == count {
                                current_interp = next_pos.coords;
                            }

                            //Checking collision with current_interp
                            let (x, y) = (current_interp.x + 1.0, current_interp.y + 1.0);
                            let index = (x as usize / cell_size as usize) as usize
                                + (y as usize / cell_size as usize) as usize * grid_w;

                            let kbots_in_proximity = &small_grid_kbot[index];

                            for kbot_id in kbots_in_proximity.iter() {
                                let kbot = kbots.get(kbot_id).unwrap();
                                let distance_to_target =
                                    (kbot.position.coords - current_interp).magnitude();

                                if distance_to_target < (kbot.radius + proj.radius) {
                                    println!("COLLISION");
                                }
                            }
                        }
                    }

                    proj.positions = proj.positions.clone().into_iter().skip(1).collect();
                    if proj.positions.len() == 0 {
                        to_remove.insert(proj.id);
                    }
                }

                for r in to_remove {
                    kinematic_projectiles.remove(&r);
                }
            }
            frame_profiler.add("4  proj move", start.elapsed());
        }

        //Projectile fire compute
        {
            let teams: HashSet<_> = players.values().map(|p| p.team).collect();

            let start2 = std::time::Instant::now();
            //TODO cache
            let mut team_to_kbots_id: HashMap<u8, HashSet<Id<KBot>>> = HashMap::new();
            let mut id_to_team: HashMap<Id<KBot>, u8> = HashMap::new();

            for team in teams.iter() {
                let team_players: Vec<_> = players.values().filter(|e| &e.team == team).collect();
                let team_kbots_id: HashSet<Id<KBot>> = team_players
                    .iter()
                    .map(|p| {
                        for kbot in p.kbots.iter() {
                            id_to_team.insert(*kbot, *team);
                        }
                        p.kbots.clone()
                    })
                    .flatten()
                    .collect();

                team_to_kbots_id.insert(*team, team_kbots_id);
            }

            let mut team_to_ennemy_grid: HashMap<u8, Vec<Vec<Id<KBot>>>> = HashMap::new();

            for team in teams.iter() {
                let grid_team: Vec<Vec<Id<KBot>>> = grid
                    .iter()
                    .map(|zone| {
                        zone.iter()
                            .filter(|kbot| id_to_team.get(kbot).unwrap() != team)
                            .copied()
                            .collect::<Vec<Id<KBot>>>()
                    })
                    .collect();

                team_to_ennemy_grid.insert(*team, grid_team);
            }

            frame_profiler.add("2  team_to_ennemy_grid", start2.elapsed());

            struct Shot {
                bot: Id<KBot>,
                target: Vector3<f32>,
            };

            let mut shots = Vec::new();

            for (me, me_kbot) in kbots.iter() {
                let grid_pos = grid_pos(me_kbot);

                let my_team = id_to_team.get(me).unwrap();

                let ennemies_in_cell = &team_to_ennemy_grid.get(my_team).unwrap()[grid_pos];

                // let mut neighbors_id: Vec<Id<KBot>> = grid[grid_pos].clone();
                // let to_remove = neighbors_id.iter().position(|e| e == me).unwrap();
                // neighbors_id.remove(to_remove);

                let can_shoot = *my_team == 0
                    && frame_count - me_kbot.frame_last_shot > me_kbot.reload_frame_count;
                if can_shoot {
                    //We choose the first ennemy in the cell, we could sort by distance or something else here
                    'meloop: for potential_ennemy in ennemies_in_cell {
                        {
                            let ennemy_kbot = kbots.get(&potential_ennemy).unwrap();

                            shots.push(Shot {
                                bot: *me,
                                target: ennemy_kbot.position.coords,
                            });
                            break 'meloop;
                        }
                    }
                }
            }

            for shot in shots.iter() {
                let kbot = kbots.get_mut(&shot.bot).unwrap();

                let dir = (shot.target - kbot.position.coords).normalize();
                let mut positions = Vec::new();
                positions.push(kbot.position + dir * kbot.radius * 1.1);

                let mut speed = dir * 2.0 + Vector3::new(0.0, 0.0, 0.2);
                for i in 0..5 {
                    speed -= Vector3::new(0.0, 0.0, 0.08);
                    positions.push(positions.last().unwrap() + speed);
                }

                kbot.frame_last_shot = frame_count;

                let proj = KinematicProjectile::new(positions);
                kinematic_projectiles.insert(proj.id, proj);
            }
        }
    }
}
