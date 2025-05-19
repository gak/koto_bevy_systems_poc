use std::{
    marker::PhantomData,
    mem::transmute,
    sync::{Arc, Mutex},
};

use bevy::{
    ecs::{system::SystemId, world::unsafe_world_cell::UnsafeWorldCell},
    log::tracing_subscriber::registry,
    platform::collections::HashMap,
    prelude::*,
};
use koto::{
    ErrorKind,
    derive::KotoType,
    prelude::*,
    runtime::{self, KFunction},
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

    app.add_systems(Startup, setup_entities);
    app.add_systems(Update, (register_koto_system_dynamic, print_entity_health));
    app.run();

    Ok(())
}

#[derive(Component, Reflect, Debug, Default, Clone, KotoType)]
#[reflect(Component, Default)]
pub(crate) struct Health(pub u32);

impl Health {}

impl KotoEntries for Health {}

impl KotoObject for Health {
    // KotoObject::Display allows mytype to be used with Koto's print function
    fn display(&self, ctx: &mut DisplayContext) -> runtime::Result<()> {
        ctx.append(format!("Health!!!({})", self.0));
        Ok(())
    }

    fn add_assign(&mut self, rhs: &KValue) -> runtime::Result<()> {
        if let KValue::Number(rhs) = rhs {
            let v: u32 = rhs.into();
            self.0 += v;
            Ok(())
        } else {
            Err(ErrorKind::UnexpectedType {
                expected: "Number".to_string(),
                unexpected: rhs.type_as_string().into(),
            }
            .into())
        }
    }
}

impl KotoCopy for Health {
    fn copy(&self) -> KObject {
        todo!()
    }
}

struct KotoSystem {
    fn_name: String,
    query_data_components: Vec<String>,
}

#[derive(Resource)]
struct Runtime {
    koto: Koto,
    systems: HashMap<String, SystemId<(), ()>>,
    koto_systems: HashMap<String, KotoSystem>,
}

impl Runtime {
    fn new() -> Self {
        Self {
            koto: Koto::new(),
            systems: HashMap::new(),
            koto_systems: HashMap::new(),
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

    let mut query_state = unsafe {
        unsafe_world_cell
            .world_mut()
            .query::<(&Name, &mut Health)>()
    };
    let query_iter = query_state.iter_mut(unsafe { unsafe_world_cell.world_mut() });
    let items: Vec<_> = query_iter.collect();
    dbg!(&items);
    let query: Vec<KValue> = items
        .into_iter()
        .map(|(name, mut health)| {
            let koto_res_mut_health = KotoBevyMut::new(&mut *health);
            KTuple::from(&[
                KValue::Str(name.to_string().into()),
                KValue::Object(koto_res_mut_health.into()),
            ])
            .into()
        })
        .collect();

    let query = KValue::List(KList::from_slice(query.as_slice()));

    match runtime.koto.call_function(my_system, query) {
        Ok(_) => {}
        Err(err) => {
            println!("{err}");
            return Err(BevyError::from(err));
        }
    }

    Ok(())
}

fn register_koto_system_dynamic(world: &mut World) -> Result<(), BevyError> {
    let unsafe_world_cell = world.as_unsafe_world_cell();

    let mut runtime = unsafe { unsafe_world_cell.world_mut().resource_mut::<Runtime>() };
    let app_type_registry = unsafe { unsafe_world_cell.world_mut().resource::<AppTypeRegistry>() };
    let type_registry = app_type_registry.read();

    let koto = &mut runtime.koto;

    let mut new_koto_systems = Arc::new(Mutex::new(Vec::<KotoSystem>::new()));
    let new_koto_systems_inner = new_koto_systems.clone();

    koto.prelude().add_fn("add_system", |ctx| {
        let mut koto_systems = new_koto_systems_inner.lock().unwrap();

        let koto_system_fn = ctx.args().first().unwrap();
        let fn_name = koto_system_fn.type_as_string().to_string();
        let KValue::Function(koto_system_fn) = koto_system_fn else {
            panic!("not a fn!");
        };

        let query_data: Vec<&KValue> = ctx.args().iter().skip(1).collect();

        let mut query_data_components = Vec::new();
        for data_item in query_data {
            let KValue::Str(s) = data_item else {
                panic!("not a str");
            };
            let component_name = s.to_string();
            query_data_components.push(component_name);
            //     let type_registration = type_registry
            //         .get_with_short_type_path(&component_name)
            //         .expect(&format!("{component_name} not in type registry"));
        }

        koto_systems.push(KotoSystem {
            fn_name,
            query_data_components,
        });

        Ok(KValue::Null)
    });

    let system_script = include_str!("system_script.koto");

    match koto.compile_and_run(system_script) {
        Ok(_) => {
            println!("ok?");
        }
        Err(err) => {
            println!("{err}");
            return Err(BevyError::from(err));
        }
    };

    Ok(())
}

#[derive(KotoType)]
struct KotoBevyMut<V>
where
    V: KotoObject + 'static,
{
    ptr: *mut V,
    _marker: PhantomData<V>,
}

unsafe impl<V: KotoObject + 'static> Send for KotoBevyMut<V> {}
unsafe impl<V: KotoObject + 'static> Sync for KotoBevyMut<V> {}

impl<V> KotoBevyMut<V>
where
    V: KotoObject + 'static,
{
    fn new(value: &mut V) -> Self {
        Self {
            ptr: value as *mut V,
            _marker: PhantomData,
        }
    }
}

impl<V> KotoObject for KotoBevyMut<V>
where
    V: KotoObject + 'static,
{
    fn display(&self, ctx: &mut DisplayContext) -> runtime::Result<()> {
        unsafe { (*self.ptr).display(ctx) }
    }
    fn add_assign(&mut self, rhs: &KValue) -> runtime::Result<()> {
        unsafe { (*self.ptr).add_assign(rhs) }
    }
    fn add(&self, rhs: &KValue) -> runtime::Result<KValue> {
        unsafe { (*self.ptr).add(rhs) }
    }
}

impl<V> KotoEntries for KotoBevyMut<V> where V: KotoObject + 'static {}

impl<V> KotoCopy for KotoBevyMut<V>
where
    V: KotoObject + 'static,
{
    fn copy(&self) -> KObject {
        todo!()
    }
}

fn print_entity_health(query: Query<(&Name, &Health)>) {
    for (name, health) in &query {
        println!("{name} {health:?}");
    }
}
