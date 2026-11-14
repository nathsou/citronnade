//! Lemon Drop: catch the lemons falling from the orchard, in stereoscopic
//! 3D. Ripe lemons score 10, golden ones 50; rotten ones cost a life, and so
//! does every good lemon that reaches the ground. The game speeds up as you
//! score. Move the crate with the Circle Pad, the D-pad or the stylus.
//!
//! A starts, SELECT toggles the autopilot, START quits. The best five
//! scores are kept on the SD card (`/citronnade/lemon-drop/scores.txt`),
//! with names typed on the software keyboard. Sound needs the DSP firmware;
//! without it the game is silent.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use citronnade::applets::{Keyboard, KeyboardButton, KeyboardKind, ValidInput};
use citronnade::audio::{Audio, SampleFormat, WaveBuf, OUTPUT_RATE};
use citronnade::gpu::{self, shader, Cull, Gpu, Mesh, MeshData, MeshVertex, RenderTarget, Shader, Source, TexEnv, TestFunc, WriteMask};
use citronnade::math::{self, Mat4, Vec3};
use citronnade::prelude::*;

const SHADER: &[u8] = shader!(
    r#"
    .fvec projection[4], modelview[4]
    .fvec light_dir
    .fvec tint
    .constf consts(0.0, 1.0, 0.45, 0.0)
    .alias inpos v0
    .alias innrm v1
    .out outpos position
    .out outclr color
    .proc main
        mov r0.xyz, inpos
        mov r0.w, consts.y
        dp4 r1.x, modelview[0], r0
        dp4 r1.y, modelview[1], r0
        dp4 r1.z, modelview[2], r0
        dp4 r1.w, modelview[3], r0
        dp4 outpos.x, projection[0], r1
        dp4 outpos.y, projection[1], r1
        dp4 outpos.z, projection[2], r1
        dp4 outpos.w, projection[3], r1
        dp3 r2.x, modelview[0], innrm
        dp3 r2.y, modelview[1], innrm
        dp3 r2.z, modelview[2], innrm
        dp3 r2.w, r2, r2
        rsq r2.w, r2.w
        mul r2.xyz, r2.xyz, r2.w
        dp3 r3.x, light_dir, r2
        max r3.x, consts.x, r3.x
        add r4, consts.zzzz, r3.xxxx
        mul outclr.xyz, tint, r4
        mov outclr.w, consts.y
        end
    .end
    "#
);

const FIELD: f32 = 3.6; // the crate moves in -FIELD..FIELD
const SCORES_FILE: &str = "/citronnade/lemon-drop/scores.txt";

// ---------------------------------------------------------------- meshes

/// A low-poly lemon: an ellipsoid along x with pointed ends.
fn lemon_mesh() -> MeshData {
    let (rows, cols) = (10, 14);
    let shape = |phi: f32, theta: f32| {
        let tip = 0.18 * (math::exp(-(phi / 0.35) * (phi / 0.35)) + math::exp(-((math::PI - phi) / 0.35) * ((math::PI - phi) / 0.35)));
        let x = math::cos(phi) * (1.0 + tip) * 0.42;
        let r = 0.3 * math::pow(math::sin(phi), 0.85);
        Vec3::new(x, r * math::cos(theta), r * math::sin(theta))
    };
    let mut m = MeshData::default();
    for i in 0..=rows {
        let phi = 0.001 + (math::PI - 0.002) * i as f32 / rows as f32;
        for j in 0..=cols {
            let theta = math::TAU * j as f32 / cols as f32;
            let p = shape(phi, theta);
            let n = (shape(phi + 0.01, theta) - shape(phi - 0.01, theta)).cross(shape(phi, theta + 0.01) - shape(phi, theta - 0.01)).normalize();
            let n = if n.dot(p) < 0.0 { -n } else { n };
            m.vertices.push(MeshVertex { position: [p.x, p.y, p.z], normal: [n.x, n.y, n.z], uv: [0.0; 2] });
        }
    }
    for i in 0..rows {
        for j in 0..cols {
            let a = (i * (cols + 1) + j) as u16;
            let b = a + cols as u16 + 1;
            m.indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    m
}

/// A unit cube centred on the origin.
fn cube_mesh() -> MeshData {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ];
    let mut m = MeshData::default();
    for (n, u, w) in faces {
        let base = m.vertices.len() as u16;
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = [0, 1, 2].map(|k| (n[k] + u[k] * su + w[k] * sv) * 0.5);
            m.vertices.push(MeshVertex { position: p, normal: n, uv: [0.0; 2] });
        }
        m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    m
}

// ---------------------------------------------------------------- sound

struct Sound {
    audio: Option<Audio>,
    catch: Arc<WaveBuf>,
    golden: Arc<WaveBuf>,
    miss: Arc<WaveBuf>,
    rotten: Arc<WaveBuf>,
    music: Arc<WaveBuf>,
    next: usize,
}

fn synth(seconds: f32, mut f: impl FnMut(f32) -> f32) -> Arc<WaveBuf> {
    let n = (seconds * OUTPUT_RATE) as usize;
    let samples: Vec<i16> = (0..n).map(|i| (f(i as f32 / OUTPUT_RATE).clamp(-1.0, 1.0) * 30000.0) as i16).collect();
    Arc::new(WaveBuf::from_samples(&samples, 1))
}

fn square(phase: f32) -> f32 {
    if math::fract(phase) < 0.5 { 1.0 } else { -1.0 }
}

impl Sound {
    fn new() -> Sound {
        let audio = Audio::new().ok();
        let catch = synth(0.14, |t| math::sin(math::TAU * (600.0 * t + 2200.0 * t * t)) * (1.0 - t / 0.14) * 0.7);
        let golden = synth(0.36, |t| {
            let f = [1046.5, 1318.5, 1568.0][((t / 0.12) as usize).min(2)];
            math::sin(math::TAU * f * t) * 0.6 * (1.0 - math::fract(t / 0.12) * 0.5)
        });
        let miss = synth(0.3, |t| square((220.0 - 400.0 * t) * t) * 0.35 * (1.0 - t / 0.3));
        let mut seed = 1u32;
        let rotten = synth(0.25, move |t| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            ((seed >> 16) as f32 / 32768.0 - 1.0) * 0.5 * (1.0 - t / 0.25)
        });
        // A four-bar loop: a gentle arpeggio over two chords.
        let notes = [261.6, 329.6, 392.0, 523.3, 392.0, 329.6, 293.7, 349.2, 440.0, 587.3, 440.0, 349.2];
        let beat = 0.25;
        let music = synth(notes.len() as f32 * beat * 2.0, move |t| {
            let k = (t / beat) as usize % notes.len();
            let local = math::fract(t / beat);
            let env = (1.0 - local).max(0.0);
            (0.6 * math::sin(math::TAU * notes[k] * t) + 0.2 * square(notes[k] * 0.5 * t)) * env * 0.22
        });
        let mut s = Sound { audio, catch, golden, miss, rotten, music, next: 1 };
        if let Some(a) = &s.audio {
            let ch = a.channel(0);
            ch.set_format(SampleFormat::Pcm16Mono);
            ch.set_sample_rate(OUTPUT_RATE);
            ch.set_volume(0.5, 0.5);
            if let Some(m) = Arc::get_mut(&mut s.music) {
                m.set_looping(true);
            }
            ch.queue(&s.music);
        }
        s
    }

    fn play(&mut self, wave: &Arc<WaveBuf>, pan: f32) {
        let Some(a) = &self.audio else { return };
        let ch = a.channel(self.next);
        self.next = if self.next >= 4 { 1 } else { self.next + 1 };
        ch.reset();
        ch.set_format(SampleFormat::Pcm16Mono);
        ch.set_sample_rate(OUTPUT_RATE);
        ch.set_volume_pan(0.9, pan);
        ch.queue(wave);
    }
}

// ---------------------------------------------------------------- scores

#[derive(Clone)]
struct Scores {
    entries: Vec<(String, u32)>,
}

impl Scores {
    fn load() -> Scores {
        let mut entries = Vec::new();
        if let Ok(text) = citronnade::fs::read_to_string(SCORES_FILE) {
            for line in text.lines() {
                if let Some((name, score)) = line.rsplit_once('\t') {
                    if let Ok(s) = score.trim().parse::<u32>() {
                        entries.push((String::from(name), s));
                    }
                }
            }
        }
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(5);
        Scores { entries }
    }

    fn qualifies(&self, score: u32) -> bool {
        score > 0 && (self.entries.len() < 5 || self.entries.iter().any(|e| score > e.1))
    }

    fn insert(&mut self, name: String, score: u32) {
        self.entries.push((name, score));
        self.entries.sort_by(|a, b| b.1.cmp(&a.1));
        self.entries.truncate(5);
    }

    fn save(&self) -> Result<()> {
        citronnade::fs::create_dir_all("/citronnade/lemon-drop")?;
        let mut text = String::new();
        for (n, s) in &self.entries {
            text.push_str(&format!("{}\t{}\n", n.replace('\t', " "), s));
        }
        citronnade::fs::write(SCORES_FILE, text.as_bytes())
    }
}

// ---------------------------------------------------------------- game

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ripe,
    Golden,
    Rotten,
}

#[derive(Clone, Copy)]
struct Lemon {
    pos: Vec3,
    vy: f32,
    spin: f32,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Title,
    Playing,
    Over { since: u32 },
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 % 100_000) as f32 / 100_000.0
    }
}

struct Game {
    state: State,
    x: f32,
    lemons: Vec<Lemon>,
    score: u32,
    lives: u32,
    caught: u32,
    until_spawn: f32,
    rng: Rng,
    autopilot: bool,
    flash: f32,
    message: String,
}

impl Game {
    fn new() -> Game {
        Game {
            state: State::Title,
            x: 0.0,
            lemons: Vec::new(),
            score: 0,
            lives: 3,
            caught: 0,
            until_spawn: 1.0,
            rng: Rng(0x5EED_1E40),
            autopilot: false,
            flash: 0.0,
            message: String::new(),
        }
    }

    fn start(&mut self) {
        self.state = State::Playing;
        self.lemons.clear();
        self.score = 0;
        self.lives = 3;
        self.caught = 0;
        self.until_spawn = 0.8;
        self.x = 0.0;
        log!("game started");
    }

    /// Gravity and spawn interval grow with the score.
    fn level(&self) -> u32 {
        1 + self.score / 150
    }

    fn spawn(&mut self) {
        let r = self.rng.next();
        let kind = if r < 0.08 { Kind::Golden } else if r < 0.25 { Kind::Rotten } else { Kind::Ripe };
        let x = (self.rng.next() * 2.0 - 1.0) * FIELD;
        self.lemons.push(Lemon { pos: Vec3::new(x, 6.5, 0.0), vy: 0.0, spin: self.rng.next() * 6.0, kind });
    }

    /// The autopilot heads for the lowest good lemon and dodges rotten ones.
    fn autopilot_target(&self) -> f32 {
        let good = self.lemons.iter().filter(|l| l.kind != Kind::Rotten).min_by(|a, b| a.pos.y.partial_cmp(&b.pos.y).unwrap());
        match good {
            Some(l) => l.pos.x,
            None => self.x,
        }
    }

    fn update(&mut self, dt: f32, steer: f32, touch_x: Option<f32>, sound: &mut Sound) {
        self.flash = (self.flash - dt).max(0.0);
        if self.state != State::Playing {
            return;
        }
        // Movement.
        let speed = 7.0;
        if self.autopilot {
            let t = self.autopilot_target();
            self.x += (t - self.x).clamp(-speed * dt, speed * dt);
        } else if let Some(tx) = touch_x {
            self.x += (tx - self.x) * (12.0 * dt).min(1.0);
        } else {
            self.x += steer * speed * dt;
        }
        self.x = self.x.clamp(-FIELD, FIELD);
        // Spawning.
        let level = self.level() as f32;
        self.until_spawn -= dt;
        if self.until_spawn <= 0.0 {
            self.spawn();
            self.until_spawn = (1.3 - level * 0.12).max(0.45) * (0.7 + self.rng.next() * 0.6);
        }
        // Falling, catching, missing.
        let gravity = 4.0 + level * 0.8;
        let x = self.x;
        let mut events = Vec::new();
        self.lemons.retain_mut(|l| {
            l.vy -= gravity * dt;
            l.pos.y += l.vy * dt;
            l.spin += dt * 3.0;
            if l.pos.y < 0.75 && l.pos.y > 0.2 && (l.pos.x - x).abs() < 0.75 {
                events.push((true, l.kind, l.pos.x));
                return false;
            }
            if l.pos.y < -0.3 {
                events.push((false, l.kind, l.pos.x));
                return false;
            }
            true
        });
        for (caught, kind, lx) in events {
            let pan = lx / FIELD;
            match (caught, kind) {
                (true, Kind::Ripe) => {
                    self.score += 10;
                    self.caught += 1;
                    sound.play(&sound.catch.clone(), pan);
                }
                (true, Kind::Golden) => {
                    self.score += 50;
                    self.caught += 1;
                    self.message = String::from("golden lemon! +50");
                    sound.play(&sound.golden.clone(), pan);
                }
                (true, Kind::Rotten) => {
                    self.lives = self.lives.saturating_sub(1);
                    self.flash = 0.3;
                    self.message = String::from("yuck, rotten!");
                    sound.play(&sound.rotten.clone(), pan);
                }
                (false, Kind::Rotten) => {}
                (false, _) => {
                    self.lives = self.lives.saturating_sub(1);
                    self.flash = 0.3;
                    self.message = String::from("splat!");
                    sound.play(&sound.miss.clone(), pan);
                }
            }
            if caught && kind != Kind::Rotten && self.caught == 1 {
                log!("caught the first lemon");
            }
        }
        if self.lives == 0 {
            self.state = State::Over { since: 0 };
            log!("game over: {} points, {} lemons caught", self.score, self.caught);
        }
    }
}

// ---------------------------------------------------------------- drawing

struct Scene {
    shader: Shader,
    lemon: Mesh,
    cube: Mesh,
    u_proj: gpu::UniformLocation,
    u_mv: gpu::UniformLocation,
    u_light: gpu::UniformLocation,
    u_tint: gpu::UniformLocation,
}

impl Scene {
    fn draw(&self, gpu: &mut Gpu, target: &RenderTarget, game: &Game, iod: f32, t: f32) -> Result<()> {
        let sky = if game.flash > 0.0 { gpu::rgba(140, 60, 60, 255) } else { gpu::rgba(120, 175, 225, 255) };
        gpu.clear(target, sky, 0)?;
        gpu.bind_target(target);
        gpu.bind_shader(&self.shader);
        gpu.set_texenv(0, TexEnv::replace(Source::PrimaryColor));
        gpu.set_depth_test(true, TestFunc::Greater, WriteMask::ALL);
        gpu.set_cull(Cull::BackCcw);
        gpu.set_blend(None);
        let projection = Mat4::perspective_stereo_tilt(math::to_radians(50.0), 400.0 / 240.0, 0.1, 60.0, iod, 9.0);
        gpu.set_mat4(self.u_proj, &projection);
        let view = Mat4::look_at(Vec3::new(0.0, 3.2, 9.0), Vec3::new(0.0, 2.2, 0.0), Vec3::new(0.0, 1.0, 0.0));
        let l = (view * Vec3::new(-0.4, 0.8, 0.5).normalize().extend(0.0)).xyz();
        gpu.set_vec4(self.u_light, [l.x, l.y, l.z, 0.0]);
        let draw = |gpu: &mut Gpu, mesh: &Mesh, model: Mat4, tint: [f32; 3]| {
            gpu.set_vec4(self.u_tint, [tint[0], tint[1], tint[2], 1.0]);
            gpu.set_mat4(self.u_mv, &(view * model));
            mesh.draw(gpu);
        };
        // Grass and the orchard behind.
        draw(gpu, &self.cube, Mat4::translation(0.0, -0.1, -4.0) * Mat4::scaling(40.0, 0.2, 24.0), [0.35, 0.62, 0.3]);
        for k in 0..6 {
            let x = -7.5 + k as f32 * 3.0;
            let z = -6.0 - (k % 2) as f32 * 2.0;
            draw(gpu, &self.cube, Mat4::translation(x, 1.0, z) * Mat4::scaling(0.4, 2.0, 0.4), [0.45, 0.3, 0.15]);
            let sway = math::sin(t * 0.8 + k as f32) * 0.05;
            draw(gpu, &self.cube, Mat4::translation(x, 2.8, z) * Mat4::rotation_z(sway) * Mat4::scaling(2.2, 1.8, 2.2), [0.25, 0.55, 0.22]);
        }
        // The crate.
        draw(gpu, &self.cube, Mat4::translation(game.x, 0.3, 0.0) * Mat4::scaling(1.4, 0.6, 0.9), [0.7, 0.5, 0.28]);
        draw(gpu, &self.cube, Mat4::translation(game.x, 0.62, 0.0) * Mat4::scaling(1.2, 0.05, 0.7), [0.5, 0.33, 0.18]);
        // Lemons.
        for lemon in &game.lemons {
            let tint = match lemon.kind {
                Kind::Ripe => [1.0, 0.86, 0.15],
                Kind::Golden => [1.0, 0.65, 0.05],
                Kind::Rotten => [0.45, 0.42, 0.2],
            };
            let s = if lemon.kind == Kind::Golden { 1.2 } else { 1.0 };
            draw(gpu, &self.lemon, Mat4::translation(lemon.pos.x, lemon.pos.y, lemon.pos.z) * Mat4::rotation_z(lemon.spin) * Mat4::rotation_y(lemon.spin * 0.7) * Mat4::scaling(s, s, s), tint);
        }
        Ok(())
    }
}

fn hud(c: &mut Canvas, game: &Game) {
    let shadow = |c: &mut Canvas, x: i32, y: i32, s: &str, color: Color, scale: u32| {
        c.text(x + 1, y + 1, s, Color::rgb(0, 0, 0), scale);
        c.text(x, y, s, color, scale);
    };
    match game.state {
        State::Title => {
            shadow(c, 92, 40, "LEMON DROP", Color::rgb(255, 230, 60), 4);
            shadow(c, 104, 90, "press A or touch to start", Color::WHITE, 1);
        }
        State::Playing | State::Over { .. } => {
            shadow(c, 8, 8, &format!("{}", game.score), Color::WHITE, 3);
            for i in 0..game.lives {
                c.fill_circle(380 - i as i32 * 18, 18, 6, Color::rgb(255, 220, 40));
            }
            if game.autopilot {
                shadow(c, 150, 8, "autopilot", Color::rgb(200, 255, 200), 1);
            }
            if let State::Over { .. } = game.state {
                shadow(c, 112, 80, "GAME OVER", Color::rgb(255, 120, 100), 3);
            }
        }
    }
}

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let _ = apt.set_speedup(true);
    let mut screens = gsp.screens()?;
    screens.set_top_mode(TopMode::Stereo);
    let mut input = hid.input()?;
    let mut gpu = Gpu::new()?;
    let target = RenderTarget::top()?;
    let shader = Shader::new(SHADER).expect("shader");
    let u = |n: &str| shader.uniform(n).unwrap();
    let (u_proj, u_mv, u_light, u_tint) = (u("projection"), u("modelview"), u("light_dir"), u("tint"));
    let scene = Scene { lemon: Mesh::from_data(&lemon_mesh())?, cube: Mesh::from_data(&cube_mesh())?, shader, u_proj, u_mv, u_light, u_tint };
    let mut sound = Sound::new();
    let mut scores = Scores::load();
    let mut game = Game::new();
    let mut last_name = String::from("PLAYER");
    log!("lemon-drop: ready ({} high scores, sound {})", scores.entries.len(), if sound.audio.is_some() { "on" } else { "off" });

    let mut frame = 0u32;
    while apt.main_loop() {
        let state = input.scan();
        let down = state.down();
        if down.contains(Buttons::START) {
            break;
        }
        if down.contains(Buttons::SELECT) {
            game.autopilot = !game.autopilot;
            log!("autopilot {}", if game.autopilot { "on" } else { "off" });
        }
        let touch = state.touch();
        match game.state {
            State::Title => {
                if down.contains(Buttons::A) || (touch.is_some() && frame > 30) {
                    game.start();
                }
            }
            State::Over { since } => {
                game.state = State::Over { since: since + 1 };
                if since == 45 && scores.qualifies(game.score) {
                    let r = Keyboard::new(KeyboardKind::Qwerty, 2, 12)
                        .hint("Your name for the high scores")
                        .initial_text(&last_name)
                        .validation(ValidInput::NotEmptyNotBlank, 0)
                        .show();
                    if r.button == Some(KeyboardButton::Right) {
                        last_name = r.text.clone();
                        scores.insert(r.text, game.score);
                        match scores.save() {
                            Ok(()) => log!("high score saved: {} {}", last_name, game.score),
                            Err(e) => log!("could not save the scores: {:?}", e),
                        }
                    }
                }
                if since > 60 && down.contains(Buttons::A) {
                    game.start();
                }
            }
            State::Playing => {}
        }
        let (cx, _) = state.circle_pad();
        let mut steer = cx as f32 / 156.0;
        if state.held().contains(Buttons::DPAD_LEFT) {
            steer = -1.0;
        }
        if state.held().contains(Buttons::DPAD_RIGHT) {
            steer = 1.0;
        }
        let touch_x = touch.map(|(tx, _)| (tx as f32 / 319.0 * 2.0 - 1.0) * FIELD);
        game.update(1.0 / 60.0, steer.clamp(-1.0, 1.0), touch_x, &mut sound);

        // The top screen, once per eye.
        let iod = citronnade::slider_3d() * 0.25;
        let t = frame as f32 / 60.0;
        scene.draw(&mut gpu, &target, &game, -iod, t)?;
        screens.show_target(&mut gpu, &target, Screen::Top)?;
        scene.draw(&mut gpu, &target, &game, iod, t)?;
        screens.show_target_right(&mut gpu, &target)?;
        {
            let (mut left, mut right) = screens.top_stereo();
            hud(&mut left, &game);
            hud(&mut right, &game);
        }

        // The bottom screen.
        let mut b = screens.bottom();
        b.vertical_gradient(0, 0, 320, 240, Color::rgb(40, 34, 20), Color::rgb(18, 16, 12));
        b.text(10, 8, "Lemon Drop", Color::rgb(255, 230, 60), 2);
        b.text(10, 30, &format!("score {}   level {}   lemons {}", game.score, game.level(), game.caught), Color::WHITE, 1);
        b.text(10, 46, &game.message, Color::rgb(255, 200, 150), 1);
        b.text(10, 70, "High scores", Color::rgb(255, 230, 60), 1);
        for (i, (n, s)) in scores.entries.iter().enumerate() {
            b.text(16, 86 + i as i32 * 12, &format!("{}. {:12} {:5}", i + 1, n, s), Color::rgb(230, 220, 200), 1);
        }
        if scores.entries.is_empty() {
            b.text(16, 86, "none yet", Color::rgb(160, 150, 130), 1);
        }
        // The touch strip: where the crate is.
        b.fill_rect(0, 196, 320, 24, Color::rgb(60, 50, 30));
        let px = ((game.x / FIELD + 1.0) / 2.0 * 319.0) as i32;
        b.fill_rect(px - 16, 198, 32, 20, Color::rgb(180, 130, 70));
        b.text(10, 226, "A start  SELECT autopilot  START quit", Color::rgb(200, 190, 170), 1);
        screens.present();
        frame += 1;
    }
    log!("bye: best score {}", scores.entries.first().map_or(0, |e| e.1));
    Ok(())
}
