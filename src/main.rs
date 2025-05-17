use std::mem::transmute;

use bevy::{
    ecs::{
        system::{RunSystemOnce, SystemId},
        world::unsafe_world_cell::UnsafeWorldCell,
    },
    platform::collections::HashMap,
    prelude::*,
};
use koto::Koto;

const SCRIPT: &str = r#"
print "koto script loaded"

call_bevy_system()

print "end koto script"
"#;

fn main() -> Result<(), BevyError> {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let mut runtime = Runtime::new();
    runtime.register_one_shot_system(app.world_mut(), "call_bevy_system", one_shot);
    app.insert_resource(runtime);

    app.world_mut().run_system_once(run_script)??;

    Ok(())
}

#[derive(Resource)]
struct Runtime {
    koto: Koto,
    systems: HashMap<String, SystemId<(), ()>>,
}

impl Runtime {
    fn new() -> Self {
        Self {
            koto: Koto::new(),
            systems: HashMap::new(),
        }
    }

    fn register_one_shot_system<M>(
        &mut self,
        world: &mut World,
        name: &str,
        system: impl IntoSystem<(), (), M> + Send + Sync + 'static,
    ) {
        let system_id = world.register_system(system);
        self.systems.insert(name.to_string(), system_id);
    }
}

fn one_shot(mut commands: Commands) {
    println!("one_shot called");

    commands.spawn_empty();
}

fn run_script(world: &mut World) -> Result<(), BevyError> {
    let unsafe_world_cell = world.as_unsafe_world_cell();
    let static_unsafe_world_cell: UnsafeWorldCell<'static> =
        unsafe { transmute(unsafe_world_cell) };

    let mut runtime = world.resource_mut::<Runtime>();

    for (name, system_id) in runtime.systems.clone() {
        runtime.koto.exports_mut().add_fn(&name, move |_ctx| {
            unsafe { static_unsafe_world_cell.world_mut().run_system(system_id) }.unwrap();
            Ok(().into())
        })
    }

    match runtime.koto.compile_and_run(SCRIPT) {
        Ok(_) => {}
        Err(err) => {
            println!("{err}");
            return Err(BevyError::from(err));
        }
    };

    Ok(())
}
