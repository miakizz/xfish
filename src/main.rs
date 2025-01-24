use fish_oxide::generate_csv;
use lambda_http::http::StatusCode;
use lambda_http::{service_fn, tracing, Error, IntoResponse, Request, RequestExt};
use std::convert::Infallible;
use std::thread;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::errors::ReplyOrIdError;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, CoordMode, CreateGCAux, CreateWindowAux, Point, PropMode, Screen, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{atom_manager, connect};

use x11rb::protocol::xproto::EventMask;

atom_manager! {
    pub Atoms: AtomsCookie {
        UTF8_STRING,
        WM_DELETE_WINDOW,
        WM_PROTOCOLS,
        _NET_WM_NAME,
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // required to enable CloudWatch error logging by the runtime
    tracing::init_default_subscriber();

    let func = service_fn(handler);
    lambda_http::run(func).await?;
    Ok(())
}

pub(crate) async fn handler(event: Request) -> Result<impl IntoResponse, Infallible> {
    match handle_response(event).await {
        Ok(res) => Ok(res.into_response().await),
        Err(err) => Ok((StatusCode::BAD_REQUEST, format!("Error: {}", err))
            .into_response()
            .await),
    }
}

pub(crate) async fn handle_response(event: Request) -> Result<impl IntoResponse, Error> {
    //Get the address of the X11 server from URL params
    let Some(mut address) = event
        .query_string_parameters_ref()
        .and_then(|params| params.first("address"))
        .and_then(|addr| Some(addr.to_string()))
    else {
        return Err("need address in query params".into());
    };

    //Similar process to check if clientside JS reported that it is 11:11
    //If param is missing, it is probably Mia testing code, so send a fish anyway
    let fish_str = match event.query_string_parameters_ref().unwrap().first("time") {
        Some("bad") => include_str!("../comeback.csv").to_owned(),
        _ => generate_csv(),
    };

    // Each row is a list of points that make up a connected line
    // Each row is not connected
    // Fish_str is CSV but it's so simple, it can be parsed manually
    let fish: Vec<Vec<Point>> = fish_str
        .split("\n")
        .map(|line| {
            // Split the line by comma, parse each item as float, then convert to i16
            line.split(',')
                .filter_map(|item| item.trim().parse::<f64>().ok().and_then(|i| Some(i as i16)))
                .collect::<Vec<i16>>() //Chunk is necessary for chunking
                .chunks(2)
                .map(|item| Point { x: item[0], y: item[1] })
                .collect()
        })
        .collect();

    //Add a default display/screen (?) number if user did not supply it
    if !address.contains(":") {
        address = address + ":0.0";
    }

    let (conn, screen_num) = connect(Some(&address))?;

    let screen = &conn.setup().roots[screen_num];
    let atoms = Atoms::new(&conn)?.reply()?;
    let win_id = create_window(&conn, screen, &atoms, (520, 320))?;
    let gc_id = conn.generate_id().unwrap();

    conn.create_gc(
        gc_id,
        win_id,
        &CreateGCAux::default()
            .foreground(screen.black_pixel)
            .graphics_exposures(0),
    )?;

    conn.flush()?;

    //Event loop time! This is a simple one as the program doesn't take user input
    loop {
        let event = conn.wait_for_event().unwrap();
        match event {
            //Window is visible, so the fish can be drawn
            Event::Expose(_event) => {
                for poly_line in &fish {
                    conn.poly_line(CoordMode::ORIGIN, win_id, gc_id, &poly_line)?;
                    //Create a slow drawing effect
                    thread::sleep(Duration::from_millis(7));
                    conn.flush()?;
                }
            }
            Event::ClientMessage(event) => {
                let data = event.data.as_data32();
                if event.format == 32 && event.window == win_id && data[0] == atoms.WM_DELETE_WINDOW {
                    println!("Window was asked to close");
                    break;
                }
            }
            Event::Error(err) => return Err(format!("Got an unexpected error: {:?}", err).into()),
            ev => println!("Got an unknown event: {:?}", ev),
        }
    }
    Ok(format!("Understandable, have a nice fish").into_response().await)
}

fn create_window(
    conn: &impl Connection,
    screen: &Screen,
    atoms: &Atoms,
    (width, height): (u16, u16),
) -> Result<Window, ReplyOrIdError> {
    let win_id = conn.generate_id()?;
    let win_aux = CreateWindowAux::new()
        .event_mask(EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY)
        .background_pixel(screen.white_pixel);

    conn.create_window(
        screen.root_depth,
        win_id,
        screen.root,
        0,
        0,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &win_aux,
    )?;

    let title = "X11:11 makeafish";
    conn.change_property8(
        PropMode::REPLACE,
        win_id,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        title.as_bytes(),
    )?;
    conn.change_property8(
        PropMode::REPLACE,
        win_id,
        atoms._NET_WM_NAME,
        atoms.UTF8_STRING,
        title.as_bytes(),
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        win_id,
        atoms.WM_PROTOCOLS,
        AtomEnum::ATOM,
        &[atoms.WM_DELETE_WINDOW],
    )?;

    conn.map_window(win_id)?;

    Ok(win_id)
}
