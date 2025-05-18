use std::mem::transmute;

use bevy::{
    ecs::{
        system::{RunSystemOnce, SystemId},
        world::unsafe_world_cell::UnsafeWorldCell,
    },
    platform::collections::HashMap,
    prelude::*,
};
use koto::{
    Koto,
    runtime::{KIterator, KIteratorOutput, KTuple, KValue, KotoIterator, MetaKey},
};

const ONE_SHOT_SCRIPT: &str = r#"
print "koto script loaded"

# call_bevy_system()

print "end koto script"
"#;

fn main() -> Result<(), BevyError> {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let mut runtime = Runtime::new();
    runtime.register_one_shot_system(app.world_mut(), "call_bevy_system", one_shot);
    app.insert_resource(runtime);

    // app.world_mut().run_system_once(run_script)??;
    //
    // app.add_systems(Startup, run_one_shot_script);
    // app.add_systems(Update, check_crash);
    // app.run();

    app.register_type::<Health>();

    app.add_systems(Startup, (setup_entities, register_koto_system).chain());
    app.run();

    Ok(())
}

#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component, Default)]
pub(crate) struct Health(pub u32);

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

fn run_one_shot_script(world: &mut World) -> Result<(), BevyError> {
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

    match runtime.koto.compile_and_run(ONE_SHOT_SCRIPT) {
        Ok(_) => {}
        Err(err) => {
            println!("{err}");
            return Err(BevyError::from(err));
        }
    };

    Ok(())
}

fn setup_entities(mut commands: Commands) {
    commands.spawn((Name::new("Player"), Health(50)));
    commands.spawn((Name::new("Monster"), Health(2000)));
}

fn register_koto_system(world: &mut World) -> Result<(), BevyError> {
    let unsafe_world_cell = world.as_unsafe_world_cell();

    let mut runtime = unsafe { unsafe_world_cell.world_mut().resource_mut::<Runtime>() };

    let system_script = include_str!("system_script.koto");

    match runtime.koto.compile_and_run(system_script) {
        Ok(_) => {}
        Err(err) => {
            println!("{err}");
            return Err(BevyError::from(err));
        }
    };

    let my_system = runtime.koto.exports().get("my_system").unwrap();

    // let s: KValue = KTuple::from(&["test".into(), 1.into()]).into();
    // let iterator = KIterator::with_std_forward_iter(
    //     vec![
    //         KIteratorOutput::Value(s.clone()),
    //         KIteratorOutput::Value(s.clone()),
    //         KIteratorOutput::Value(s.clone()),
    //         KIteratorOutput::Value(s.clone()),
    //     ]
    //     .into_iter(),
    // );

    let mut query_state = unsafe { unsafe_world_cell.world_mut().query::<&Health>() };
    let query_iter = query_state.iter(unsafe { unsafe_world_cell.world_mut() });
    let items: Vec<_> = query_iter.collect();
    dbg!(items);

    // let query = KValue::Iterator(iterator);

    // match runtime.koto.call_function(my_system, query) {
    //     Ok(_) => {}
    //     Err(err) => {
    //         println!("{err}");
    //         return Err(BevyError::from(err));
    //     }
    // }

    Ok(())
}
