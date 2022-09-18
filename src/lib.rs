use std::collections::HashMap;
use std::slice;
use serde::{Serialize, Deserialize};
use serde_wasm_bindgen::{from_value, to_value};

use js_sys::{ArrayIter, Object};
use log::*;
use screeps::{
    find, game, prelude::*, Creep, ObjectId, Part, ResourceType, ReturnCode, RoomObjectProperties,
    Source, StructureController, StructureObject, StructureSpawn, Structure, RawObjectId, JsHashMap, memory, StructureType, SpawnOptions, StoreObject, StructureExtension, ConstructionSite
};
use wasm_bindgen::{prelude::*, JsCast};

mod logging;

// add wasm_bindgen to any function you would like to expose for call from js
#[wasm_bindgen]
pub fn setup() {
    logging::setup_logging(logging::Info);
}

// // this is one way to persist data between ticks within Rust's memory, as opposed to
// // keeping state in memory on game objects - but will be lost on global resets!
// thread_local! {
//     static CREEP_TARGETS: RefCell<HashMap<String, CreepTarget>> = RefCell::new(HashMap::new());
// }

// this enum will represent a creep's lock on a specific target object, storing a js reference to the object id so that we can grab a fresh reference to the object each successive tick, since screeps game objects become 'stale' and shouldn't be used beyond the tick they were fetched
// #[derive(Clone)]
// enum CreepTarget {
//     Upgrade(ObjectId<StructureController>),
//     Harvest(ObjectId<Source>),
//     Transfer(ObjectId<StructureSpawn>),
// }

#[derive(Clone, Serialize, Deserialize)]
enum StructureMemory {
    GenericSpawner(i32),
}

#[derive(Clone, Serialize, Deserialize)]
enum CreepMemory {
    SimpleWorker(SimpleJob)
}

#[derive(Clone, Serialize, Deserialize)]
enum SimpleJob {
    MoveToSource(ObjectId<Source>),
    HarvestSource(ObjectId<Source>),
    MoveToController(ObjectId<StructureController>),
    UpgradeController(ObjectId<StructureController>),
    MoveToSpawn(ObjectId<StructureSpawn>),
    TransferToSpawn(ObjectId<StructureSpawn>),
    MoveToExtension(ObjectId<StructureExtension>),
    TransferToExtension(ObjectId<StructureExtension>),
    MoveToConstructionSite(ObjectId<ConstructionSite>),
    ConstructSite(ObjectId<ConstructionSite>),
    Idle,
}

fn run_structures(structures: &JsHashMap<RawObjectId, StructureObject>) {
    structures.values().for_each(|structure| {
        run_structure(structure);
    });
}

fn run_structure(structure: StructureObject) {
    match structure.structure_type() {
        StructureType::Spawn => run_spawn(structure.as_structure().to_owned().unchecked_into()),
        StructureType::Controller => run_controller(structure.as_structure().to_owned().unchecked_into()),
        StructureType::Extension => {},
        st => warn!("Could not run structure of type {:?}", st),
    }
}

fn spawn_simple_worker(spawn: &StructureSpawn, name: &str) {
    spawn.spawn_creep_with_options(
        &[Part::Move, Part::Carry, Part::Work],
        name,
        &SpawnOptions::new().memory(to_value(&CreepMemory::SimpleWorker(SimpleJob::Idle)).unwrap()));
}

struct Task {
    /// The judge of suitability of a creep for the task.
    suitability: fn(&Task, &Creep) -> fn(i32) -> i32,
    /// The Ids of all creeps associated with this task.
    creeps: Vec<ObjectId<Creep>>,
    /// The target fulfillment of the task.
    target: i32,
    /// The execution function for a creep.
    executor: fn(&Creep) -> (),
}



fn run_spawn(spawn: StructureSpawn) {
    match (spawn.name().as_string().unwrap().as_str(), from_value(spawn.memory())) {
        (_, Ok(StructureMemory::GenericSpawner(n))) => {
            if spawn.store().get_used_capacity(Some(ResourceType::Energy)) >= 200 {
                let name = format!("{}.Simple.{}", spawn.name().as_string().unwrap(), n);
                debug!("Spawning a new Simple worker: {}", name);
                spawn_simple_worker(&spawn, &name);
                spawn.set_memory(&to_value(&StructureMemory::GenericSpawner(n + 1)).unwrap());
            }
        },
        // Bootstrap the initial spawner into role-based management
        ("Spawn1", Err(_)) => {
            let mem = StructureMemory::GenericSpawner(0);
            spawn.set_memory(&to_value(&mem).unwrap());
        },
        _ => debug!("FUCK"),
    }
}

fn run_controller(controller: StructureController) {}

fn run_creeps(creeps: &JsHashMap<String, Creep>) {
    creeps.values().for_each(|creep| {
        run_creep(creep);
    });
}

fn run_creep(creep: Creep) {
    let mem: CreepMemory = from_value(creep.memory()).unwrap_or(CreepMemory::SimpleWorker(SimpleJob::Idle));
    match mem {
        CreepMemory::SimpleWorker(job) => {
            run_simple_worker_with_job(&creep, &job);
        }
    }
}

fn run_simple_worker_with_job(creep: &Creep, job: &SimpleJob) {
    match job {
        &SimpleJob::TransferToSpawn(spawn_id) => {
            let spawn = spawn_id.resolve().expect(format!("Couldn't resolve spawn: {}", spawn_id).as_str());
            if creep.pos().is_near_to(spawn.pos()) {
                creep.transfer(&spawn, ResourceType::Energy, None);
                let source = &creep.room().expect("creep isn't in a room?").find(find::SOURCES)[0];
                creep.set_memory(&to_value(&SimpleJob::MoveToSource(source.id())).unwrap());
            }
        },
        &SimpleJob::HarvestSource(source_id) => {
            let source = source_id.resolve().expect(format!("Couldn't resolve source: {}", source_id).as_str());
            if creep.pos().is_near_to(source.pos()) {
                if creep.store().get_free_capacity(Some(ResourceType::Energy)) > 0 {
                    creep.harvest(&source);
                    // Don't transition
                } else {
                    let room = creep.room().unwrap();
                    if let Some(site) = room.find(find::MY_CONSTRUCTION_SITES).first() {
                        creep.set_memory(&to_value(&SimpleJob::MoveToConstructionSite(site.try_id().unwrap())).unwrap());
                    } else if let Some(spawn) = room.find(find::MY_SPAWNS).first() {
                        creep.set_memory(&to_value(&SimpleJob::MoveToSpawn(spawn.id())).unwrap());
                    } else {
                        let controller_id = creep.room().unwrap().controller().unwrap().id();
                        creep.set_memory(&to_value(&SimpleJob::MoveToController(controller_id)).unwrap());
                    }
                }
            } else {
                creep.set_memory(&to_value(&SimpleJob::Idle).unwrap())
            }
        },
        &SimpleJob::MoveToConstructionSite(construction_site_id) => {
            if let Some(construction_site) = construction_site_id.resolve() {
                if creep.pos().is_near_to(construction_site.pos()) {
                    creep.set_memory(&to_value(&SimpleJob::ConstructSite(construction_site_id)).unwrap());
                } else {
                    creep.move_to(construction_site);
                    // Don't transition
                }
            } else {
                warn!("Could not complete path to Construction Site: {}", construction_site_id);
                creep.set_memory(&to_value(&SimpleJob::Idle).unwrap());
            }
        },
        &SimpleJob::MoveToController(controller_id) => {
            if let Some(controller) = controller_id.resolve() {
                if creep.pos().is_near_to(controller.pos()) {
                    creep.set_memory(&to_value(&SimpleJob::UpgradeController(controller_id)).unwrap());
                } else {
                    creep.move_to(controller);
                    // Don't transition
                }
            } else {
                warn!("Could not complete path to Controller: {}", controller_id);
                creep.set_memory(&to_value(&SimpleJob::Idle).unwrap());
            }
        },
        // and so on, until you do everything.
    }
}

// to use a reserved name as a function name, use `js_name`:
#[wasm_bindgen(js_name = loop)]
pub fn game_loop() {
    debug!("loop starting! CPU: {}", game::cpu::get_used());
    let structures = game::structures();
    let creeps = game::creeps();
    run_structures(&structures);
    run_creeps(&creeps);
    debug!("running spawns");
}
