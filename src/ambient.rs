//! A few words about the moment: the weekday, the part of the day, the month,
//! and the weather, as a description to hand to Jev.
//!
//! [`describe`] is plain text in, text out. Looking the weather up needs the
//! network and so is behind the `generation` feature: [`Ambient`] does it on a
//! worker thread, with [Open-Meteo](https://open-meteo.com) for the forecast
//! (no key needed) and `ipwho.is` to find roughly where the machine is, unless
//! a place is named. Nothing here knows about Jev or gradients.
use chrono::{Datelike, NaiveDateTime, Timelike};
use serde::Deserialize;

/// The weather where, and when, it was asked about.
#[derive(Clone, Debug, PartialEq)]
pub struct Weather {
    /// The local time at that place when the forecast was made.
    pub now: NaiveDateTime,
    pub latitude: f64,
    pub temperature_c: f64,
    /// A WMO weather code, as in [`weather_words`].
    pub code: u8,
    pub wind_kmh: f64,
    /// Today's sunrise and sunset, absent where the sun does not rise or set.
    pub sunrise: Option<NaiveDateTime>,
    pub sunset: Option<NaiveDateTime>,
}

const TIME_FORMAT: &str = "%Y-%m-%dT%H:%M";

#[derive(Deserialize)]
struct Forecast {
    latitude: f64,
    current: Current,
    daily: Option<Daily>,
}

#[derive(Deserialize)]
struct Current {
    time: String,
    temperature_2m: f64,
    weather_code: u8,
    wind_speed_10m: Option<f64>,
}

#[derive(Deserialize)]
struct Daily {
    #[serde(default)]
    sunrise: Vec<Option<String>>,
    #[serde(default)]
    sunset: Vec<Option<String>>,
}

/// The URL that asks Open-Meteo for the weather now, and today's sun times in
/// the local time zone of the place.
pub fn forecast_url(latitude: f64, longitude: f64) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={latitude:.4}&longitude={longitude:.4}\
         &current=temperature_2m,weather_code,wind_speed_10m\
         &daily=sunrise,sunset&forecast_days=1&timezone=auto"
    )
}

/// Read an Open-Meteo forecast response (see [`forecast_url`]).
pub fn parse_forecast(json: &str) -> Result<Weather, String> {
    let forecast: Forecast =
        serde_json::from_str(json).map_err(|_| "Unexpected weather response".to_string())?;
    let when = |text: &str| NaiveDateTime::parse_from_str(text, TIME_FORMAT).ok();
    let now = when(&forecast.current.time).ok_or("Unexpected time in weather response")?;
    let first = |times: Option<&Vec<Option<String>>>| times?.first()?.as_deref().and_then(when);
    let daily = forecast.daily.as_ref();
    let temperature_c = forecast.current.temperature_2m;
    if !forecast.latitude.is_finite() || !temperature_c.is_finite() {
        return Err("Unexpected numbers in weather response".into());
    }
    Ok(Weather {
        now,
        latitude: forecast.latitude,
        temperature_c,
        code: forecast.current.weather_code,
        wind_kmh: forecast.current.wind_speed_10m.unwrap_or(0.0),
        sunrise: first(daily.map(|d| &d.sunrise)),
        sunset: first(daily.map(|d| &d.sunset)),
    })
}

/// The description for a WMO weather code, or `None` for one this does not know.
pub fn weather_words(code: u8) -> Option<&'static str> {
    Some(match code {
        0 => "clear sky",
        1 => "mostly clear",
        2 => "partly cloudy",
        3 => "cloudy",
        45 | 48 => "fog",
        51 => "light drizzle",
        53 => "drizzle",
        55 => "heavy drizzle",
        56 | 57 => "freezing drizzle",
        61 => "light rain",
        63 => "rain",
        65 => "heavy rain",
        66 | 67 => "freezing rain",
        71 => "light snow",
        73 => "snow",
        75 => "heavy snow",
        77 => "snow grains",
        80 => "light showers",
        81 => "showers",
        82 => "violent showers",
        85 | 86 => "snow showers",
        95 => "thunderstorm",
        96 | 99 => "thunderstorm with hail",
        _ => return None,
    })
}

fn temperature_words(celsius: f64) -> &'static str {
    match celsius {
        t if t <= -10.0 => "bitterly cold",
        t if t <= 0.0 => "freezing",
        t if t <= 8.0 => "cold",
        t if t <= 15.0 => "cool",
        t if t <= 22.0 => "mild",
        t if t <= 28.0 => "warm",
        t if t <= 34.0 => "hot",
        _ => "scorching",
    }
}

fn wind_words(kmh: f64) -> Option<&'static str> {
    match kmh {
        w if w >= 40.0 => Some("windy"),
        w if w >= 25.0 => Some("breezy"),
        _ => None,
    }
}

/// The season at `latitude` in `month` (1 to 12), or `None` in the tropics,
/// where there is not much of one.
fn season(month: u32, latitude: f64) -> Option<&'static str> {
    if latitude.abs() < 23.5 {
        return None;
    }
    let northern = match month {
        12 | 1 | 2 => "winter",
        3..=5 => "spring",
        6..=8 => "summer",
        _ => "autumn",
    };
    Some(match (latitude > 0.0, northern) {
        (true, season) => season,
        (false, "winter") => "summer",
        (false, "summer") => "winter",
        (false, "spring") => "autumn",
        (false, _) => "spring",
    })
}

/// Where in the day `now` falls. With today's sunrise and sunset, dawn, golden
/// hour, dusk and night follow the sun; without them, the clock.
fn day_part(now: NaiveDateTime, sun: Option<(NaiveDateTime, NaiveDateTime)>) -> &'static str {
    let hour = now.hour();
    if let Some((sunrise, sunset)) = sun {
        let since_sunrise = (now - sunrise).num_minutes();
        let since_sunset = (now - sunset).num_minutes();
        if since_sunrise < -60 {
            return "late night";
        } else if since_sunrise < 30 {
            return "dawn";
        } else if since_sunset >= 60 {
            return if hour >= 21 { "night" } else { "evening" };
        } else if since_sunset >= -20 {
            return "dusk";
        } else if since_sunset >= -75 {
            return "golden hour";
        }
        return match hour {
            0..=10 => "morning",
            11..=13 => "midday",
            _ => "afternoon",
        };
    }
    match hour {
        0..=4 => "late night",
        5..=6 => "dawn",
        7..=10 => "morning",
        11..=13 => "midday",
        14..=17 => "afternoon",
        18..=20 => "evening",
        _ => "night",
    }
}

/// A line about the moment, such as "Friday golden hour in October (autumn),
/// light rain, cool". `local_now` is the machine's own time, used when there is
/// no `weather`; with it, the time at the weather's place is used, so the sun
/// and the clock agree. A place's name is never included.
pub fn describe(local_now: NaiveDateTime, weather: Option<&Weather>) -> String {
    let now = weather.map_or(local_now, |weather| weather.now);
    let sun = weather.and_then(|w| Some((w.sunrise?, w.sunset?)));
    let mut text = format!(
        "{} {} in {}",
        now.format("%A"),
        day_part(now, sun),
        now.format("%B")
    );
    let Some(weather) = weather else {
        return text;
    };
    if let Some(season) = season(now.month(), weather.latitude) {
        text.push_str(&format!(" ({season})"));
    }
    let wind = wind_words(weather.wind_kmh);
    let details = [
        weather_words(weather.code),
        Some(temperature_words(weather.temperature_c)),
        wind,
    ];
    for detail in details.into_iter().flatten() {
        text.push_str(", ");
        text.push_str(detail);
    }
    text
}

/// Just the light and the weather, such as "golden hour, light rain": what is
/// left of [`describe`] when the moment is only to tint something else. Without
/// a `weather`, the part of the day alone.
pub fn describe_briefly(local_now: NaiveDateTime, weather: Option<&Weather>) -> String {
    let now = weather.map_or(local_now, |weather| weather.now);
    let sun = weather.and_then(|w| Some((w.sunrise?, w.sunset?)));
    let mut text = day_part(now, sun).to_owned();
    if let Some(words) = weather.and_then(|weather| weather_words(weather.code)) {
        text.push_str(", ");
        text.push_str(words);
    }
    text
}

/// The moment, in full and in brief.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moment {
    /// As [`describe`] gives it.
    pub full: String,
    /// As [`describe_briefly`] gives it.
    pub brief: String,
}

impl Moment {
    pub fn new(local_now: NaiveDateTime, weather: Option<&Weather>) -> Self {
        Self {
            full: describe(local_now, weather),
            brief: describe_briefly(local_now, weather),
        }
    }
}

#[cfg(feature = "generation")]
pub use lookup::{Ambient, Place};

#[cfg(feature = "generation")]
mod lookup {
    use std::{
        sync::mpsc::{self, Receiver, TryRecvError},
        time::{Duration, Instant},
    };

    use chrono::Local;
    use serde::Deserialize;

    use super::{Moment, Weather, forecast_url, parse_forecast};

    /// How long [`Ambient::poll`] waits for the network before settling for a
    /// description of the time alone.
    const WAIT: Duration = Duration::from_secs(3);
    const REQUEST_TIMEOUT: Duration = Duration::from_millis(2500);
    /// Refresh the weather context while the timer is running.
    const REFRESH: Duration = Duration::from_secs(5 * 60);

    /// Whose weather to describe.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Place {
        /// Wherever this machine's IP address seems to be.
        Here,
        /// A place by name, such as "Lisbon".
        Named(String),
    }

    #[derive(Deserialize)]
    struct Located {
        success: Option<bool>,
        latitude: Option<f64>,
        longitude: Option<f64>,
    }

    #[derive(Deserialize)]
    struct Geocoded {
        results: Option<Vec<Located>>,
    }

    fn percent_encode(text: &str) -> String {
        let mut out = String::new();
        for byte in text.bytes() {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                out.push(byte as char);
            } else {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
        out
    }

    fn coordinates(located: &Located) -> Result<(f64, f64), String> {
        match (located.success, located.latitude, located.longitude) {
            (Some(false), ..) => Err("Could not find a location".into()),
            (_, Some(latitude), Some(longitude))
                if latitude.abs() <= 90.0 && longitude.abs() <= 180.0 =>
            {
                Ok((latitude, longitude))
            }
            _ => Err("Could not find a location".into()),
        }
    }

    /// Resolve a place to coordinates, with `get` fetching a URL's text.
    fn locate(
        place: &Place,
        get: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<(f64, f64), String> {
        Ok(match place {
            Place::Here => {
                let located: Located = serde_json::from_str(&get("https://ipwho.is/")?)
                    .map_err(|_| "Unexpected location response".to_string())?;
                coordinates(&located)?
            }
            Place::Named(name) => {
                let url = format!(
                    "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&format=json",
                    percent_encode(name.trim())
                );
                let geocoded: Geocoded = serde_json::from_str(&get(&url)?)
                    .map_err(|_| "Unexpected location response".to_string())?;
                let first = geocoded
                    .results
                    .and_then(|results| results.into_iter().next())
                    .ok_or(format!("No place called '{name}'"))?;
                coordinates(&first)?
            }
        })
    }

    /// Fetch a forecast for already-resolved coordinates.
    fn forecast_at(
        latitude: f64,
        longitude: f64,
        get: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<Weather, String> {
        parse_forecast(&get(&forecast_url(latitude, longitude))?)
    }

    /// The weather at `place`, with `get` fetching a URL's text.
    #[cfg(test)]
    fn weather_at(
        place: &Place,
        get: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<Weather, String> {
        let (latitude, longitude) = locate(place, get)?;
        forecast_at(latitude, longitude, get)
    }

    fn weather_with_coordinates(
        place: &Place,
        get: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<(Weather, (f64, f64)), String> {
        let coordinates = locate(place, get)?;
        let weather = forecast_at(coordinates.0, coordinates.1, get)?;
        Ok((weather, coordinates))
    }

    fn forecast_with_coordinates(
        coordinates: (f64, f64),
        get: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<Weather, String> {
        forecast_at(coordinates.0, coordinates.1, get)
    }

    fn http_get(url: &str) -> Result<String, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .max_redirects(0)
            .build()
            .into();
        let mut response = agent
            .get(url)
            .call()
            .map_err(|_| "Could not reach the weather service".to_string())?;
        response
            .body_mut()
            .with_config()
            .limit(64 * 1024)
            .read_to_string()
            .map_err(|_| "Could not read the weather response".to_string())
    }

    /// Finds the weather on a worker thread and refreshes the moment every
    /// [`REFRESH`]. Location coordinates are kept between forecast requests.
    /// If a request fails, the latest moment falls back to local time alone.
    pub struct Ambient {
        place: Option<Place>,
        coordinates: Option<(f64, f64)>,
        started: Instant,
        next_refresh: Instant,
        receive: Option<Receiver<Result<(Weather, (f64, f64)), String>>>,
        initial_pending: bool,
    }

    impl Ambient {
        pub fn start(place: Option<Place>) -> Self {
            let started = Instant::now();
            let receive = place
                .as_ref()
                .and_then(|place| Self::request(place.clone(), None));
            Self {
                place,
                coordinates: None,
                started,
                next_refresh: started + REFRESH,
                receive,
                initial_pending: true,
            }
        }

        fn request(
            place: Place,
            coordinates: Option<(f64, f64)>,
        ) -> Option<Receiver<Result<(Weather, (f64, f64)), String>>> {
            let (send, receive) = mpsc::channel();
            std::thread::Builder::new()
                .name("ambient-weather".into())
                .spawn(move || {
                    let result = match coordinates {
                        Some(coordinates) => forecast_with_coordinates(coordinates, &http_get)
                            .map(|weather| (weather, coordinates)),
                        None => weather_with_coordinates(&place, &http_get),
                    };
                    let _ = send.send(result);
                })
                .ok()?;
            Some(receive)
        }

        /// Whether [`poll`](Self::poll) still has a description to give.
        pub fn is_pending(&self) -> bool {
            self.initial_pending
        }

        /// Return an updated moment when the initial lookup finishes and then
        /// every five minutes. This is polled from the timer's event loop.
        pub fn poll(&mut self) -> Option<Moment> {
            if let Some(receive) = self.receive.as_ref() {
                match receive.try_recv() {
                    Ok(Ok((weather, coordinates))) => {
                        self.coordinates = Some(coordinates);
                        self.receive = None;
                        self.initial_pending = false;
                        self.next_refresh = self.started + REFRESH;
                        return Some(Moment::new(Local::now().naive_local(), Some(&weather)));
                    }
                    Ok(Err(_)) => {
                        self.receive = None;
                        self.initial_pending = false;
                        self.next_refresh = self.started + REFRESH;
                        return Some(Moment::new(Local::now().naive_local(), None));
                    }
                    Err(TryRecvError::Empty) if self.started.elapsed() < WAIT => return None,
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                        self.receive = None;
                        self.initial_pending = false;
                        self.next_refresh = self.started + REFRESH;
                        return Some(Moment::new(Local::now().naive_local(), None));
                    }
                }
            }

            if self.initial_pending {
                self.initial_pending = false;
                return Some(Moment::new(Local::now().naive_local(), None));
            }

            if Instant::now() >= self.next_refresh {
                self.started = Instant::now();
                self.next_refresh = self.started + REFRESH;
                if let Some(place) = self.place.clone() {
                    self.receive = Self::request(place, self.coordinates);
                    if self.receive.is_some() {
                        return None;
                    }
                }
                return Some(Moment::new(Local::now().naive_local(), None));
            }

            None
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const FORECAST: &str = r#"{"latitude":30.27,"longitude":-97.75,
            "current":{"time":"2026-10-09T16:45","temperature_2m":32.4,
            "weather_code":0,"wind_speed_10m":10.8},
            "daily":{"sunrise":["2026-10-09T07:29"],"sunset":["2026-10-09T19:06"]}}"#;

        fn canned(url: &str) -> Result<String, String> {
            if url.starts_with("https://ipwho.is") {
                Ok(r#"{"success":true,"latitude":30.27,"longitude":-97.75}"#.into())
            } else if url.contains("geocoding-api.open-meteo.com")
                && url.contains("name=S%C3%A3o%20Paulo")
            {
                Ok(r#"{"results":[{"latitude":-23.5,"longitude":-46.6}]}"#.into())
            } else if url.contains("geocoding-api.open-meteo.com") {
                Ok("{}".into())
            } else if url.starts_with("https://api.open-meteo.com/v1/forecast?latitude=30.2700") {
                Ok(FORECAST.into())
            } else {
                Err(format!("unexpected request: {url}"))
            }
        }

        #[test]
        fn here_looks_up_the_ip_then_the_forecast() {
            let weather = weather_at(&Place::Here, &canned).unwrap();
            assert_eq!(weather.code, 0);
        }

        #[test]
        fn a_named_place_is_geocoded_with_its_name_encoded() {
            // São Paulo is geocoded; the forecast for it is not canned, which
            // shows the coordinates from the geocoder were used.
            let error = weather_at(&Place::Named(" São Paulo ".into()), &canned).unwrap_err();
            assert!(
                error.contains("latitude=-23.5000&longitude=-46.6000"),
                "{error}"
            );
            let error = weather_at(&Place::Named("Nowhere".into()), &canned).unwrap_err();
            assert_eq!(error, "No place called 'Nowhere'");
        }

        #[test]
        fn failures_of_every_kind_are_errors() {
            let failing = |_: &str| Err::<String, String>("offline".into());
            assert_eq!(weather_at(&Place::Here, &failing).unwrap_err(), "offline");
            let garbage = |_: &str| Ok::<String, String>("<html>".into());
            assert!(weather_at(&Place::Here, &garbage).is_err());
            let refused = |_: &str| Ok::<String, String>(r#"{"success":false}"#.into());
            assert!(weather_at(&Place::Here, &refused).is_err());
            let out_of_range =
                |_: &str| Ok::<String, String>(r#"{"latitude":95,"longitude":0}"#.into());
            assert!(weather_at(&Place::Here, &out_of_range).is_err());
        }

        #[test]
        fn encoding_keeps_unreserved_characters_only() {
            assert_eq!(percent_encode("Aa0-_.~ /é&"), "Aa0-_.~%20%2F%C3%A9%26");
        }

        #[test]
        fn without_a_place_the_first_poll_answers_with_the_time_alone() {
            let mut ambient = Ambient::start(None);
            assert!(ambient.is_pending());
            let moment = ambient.poll().unwrap();
            assert!(!moment.full.contains(','), "{}", moment.full);
            assert_eq!(moment.brief, moment.brief.trim());
            assert!(!moment.brief.contains(','), "{}", moment.brief);
            assert!(!ambient.is_pending());
            assert_eq!(ambient.poll(), None);
        }

        #[test]
        fn a_slow_lookup_is_waited_for_and_then_given_up_on() {
            let (_send, receive) = mpsc::channel();
            let mut ambient = Ambient {
                place: Some(Place::Here),
                coordinates: None,
                started: Instant::now(),
                next_refresh: Instant::now() + REFRESH,
                receive: Some(receive),
                initial_pending: true,
            };
            assert_eq!(ambient.poll(), None);
            ambient.started -= WAIT;
            assert!(ambient.poll().is_some());
            assert!(!ambient.is_pending());
        }

        #[test]
        fn a_lookup_that_arrives_is_described_with_its_weather() {
            let (send, receive) = mpsc::channel();
            send.send(parse_forecast(FORECAST).map(|weather| (weather, (30.27, -97.75))))
                .unwrap();
            let mut ambient = Ambient {
                place: Some(Place::Here),
                coordinates: None,
                started: Instant::now(),
                next_refresh: Instant::now() + REFRESH,
                receive: Some(receive),
                initial_pending: true,
            };
            assert_eq!(
                ambient.poll().unwrap(),
                Moment {
                    full: "Friday afternoon in October (autumn), clear sky, hot".into(),
                    brief: "afternoon, clear sky".into(),
                }
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, TIME_FORMAT).unwrap()
    }

    fn weather() -> Weather {
        Weather {
            now: time("2026-10-09T16:45"),
            latitude: 40.0,
            temperature_c: 12.0,
            code: 61,
            wind_kmh: 5.0,
            sunrise: Some(time("2026-10-09T07:30")),
            sunset: Some(time("2026-10-09T19:00")),
        }
    }

    #[test]
    fn the_brief_moment_is_only_the_light_and_the_weather() {
        // Golden hour by the sun, rain by the code; no weekday, month, season,
        // temperature or wind.
        assert_eq!(
            describe_briefly(time("2026-10-09T12:00"), Some(&weather())),
            "afternoon, light rain"
        );
        let mut windy = weather();
        windy.now = time("2026-10-09T18:00");
        windy.wind_kmh = 50.0;
        windy.temperature_c = -5.0;
        assert_eq!(
            describe_briefly(time("2026-10-09T12:00"), Some(&windy)),
            "golden hour, light rain"
        );
        // With no weather it is the clock's part of the day, in the local time.
        assert_eq!(describe_briefly(time("2026-10-09T08:00"), None), "morning");
        let mut unknown = weather();
        unknown.code = 200;
        assert_eq!(
            describe_briefly(time("2026-10-09T12:00"), Some(&unknown)),
            "afternoon"
        );
        let moment = Moment::new(time("2026-10-09T08:00"), None);
        assert_eq!(moment.brief, "morning");
        assert_eq!(moment.full, describe(time("2026-10-09T08:00"), None));
    }

    #[test]
    fn parses_an_open_meteo_response() {
        let json = r#"{"latitude":30.269146,"longitude":-97.75338,"utc_offset_seconds":-18000,
            "current":{"time":"2026-10-09T16:45","interval":900,"temperature_2m":32.4,
            "weather_code":0,"is_day":1,"wind_speed_10m":10.8},
            "daily":{"time":["2026-10-09"],"sunrise":["2026-10-09T07:29"],"sunset":["2026-10-09T19:06"]}}"#;
        let weather = parse_forecast(json).unwrap();
        assert_eq!(weather.now, time("2026-10-09T16:45"));
        assert_eq!(weather.sunrise, Some(time("2026-10-09T07:29")));
        assert_eq!(weather.sunset, Some(time("2026-10-09T19:06")));
        assert_eq!(
            (weather.code, weather.temperature_c, weather.wind_kmh),
            (0, 32.4, 10.8)
        );
    }

    #[test]
    fn missing_sun_times_and_wind_are_allowed_but_missing_weather_is_not() {
        let polar = r#"{"latitude":78.2,"current":{"time":"2026-06-21T12:00",
            "temperature_2m":4.0,"weather_code":3},
            "daily":{"sunrise":[null],"sunset":[null]}}"#;
        let weather = parse_forecast(polar).unwrap();
        assert_eq!(
            (weather.sunrise, weather.sunset, weather.wind_kmh),
            (None, None, 0.0)
        );
        let bare = r#"{"latitude":1.0,"current":{"time":"2026-06-21T12:00",
            "temperature_2m":4.0,"weather_code":3}}"#;
        assert!(parse_forecast(bare).is_ok());
        for bad in [
            "",
            "{}",
            r#"{"latitude":1.0,"current":{"time":"yesterday","temperature_2m":4.0,"weather_code":3}}"#,
            r#"{"latitude":1.0,"current":{"time":"2026-06-21T12:00","weather_code":3}}"#,
            r#"{"latitude":1.0,"current":{"time":"2026-06-21T12:00","temperature_2m":4.0,"weather_code":300}}"#,
        ] {
            assert!(parse_forecast(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn describes_the_moment_with_weather() {
        assert_eq!(
            describe(time("2000-01-01T00:00"), Some(&weather())),
            "Friday afternoon in October (autumn), light rain, cool"
        );
        let mut windy = weather();
        windy.wind_kmh = 45.0;
        windy.code = 200; // Not a code this knows: it is left out.
        assert_eq!(
            describe(time("2000-01-01T00:00"), Some(&windy)),
            "Friday afternoon in October (autumn), cool, windy"
        );
    }

    #[test]
    fn without_weather_it_is_the_local_clock_alone() {
        assert_eq!(
            describe(time("2026-03-01T08:15"), None),
            "Sunday morning in March"
        );
    }

    #[test]
    fn seasons_flip_in_the_south_and_vanish_in_the_tropics() {
        assert_eq!(season(10, 40.0), Some("autumn"));
        assert_eq!(season(10, -34.0), Some("spring"));
        assert_eq!(season(1, 51.0), Some("winter"));
        assert_eq!(season(1, -33.0), Some("summer"));
        assert_eq!(season(7, 10.0), None);
        assert_eq!(season(7, -23.4), None);
    }

    #[test]
    fn the_day_follows_the_sun_when_it_is_known() {
        let sun = Some((time("2026-10-09T07:30"), time("2026-10-09T19:00")));
        let part = |at| day_part(time(&format!("2026-10-09T{at}")), sun);
        assert_eq!(part("05:00"), "late night");
        assert_eq!(part("06:30"), "dawn");
        assert_eq!(part("07:50"), "dawn");
        assert_eq!(part("09:00"), "morning");
        assert_eq!(part("12:00"), "midday");
        assert_eq!(part("15:00"), "afternoon");
        assert_eq!(part("17:45"), "golden hour");
        assert_eq!(part("18:50"), "dusk");
        assert_eq!(part("19:30"), "dusk");
        assert_eq!(part("20:30"), "evening");
        assert_eq!(part("22:00"), "night");
        // Summer in the far north: the sun is up all day, and the clock rules.
        assert_eq!(day_part(time("2026-06-21T23:30"), None), "night");
    }

    #[test]
    fn the_day_follows_the_clock_without_the_sun() {
        let part = |hour| day_part(time(&format!("2026-10-09T{hour:02}:00")), None);
        let parts: Vec<_> = (0..24).map(part).collect();
        assert_eq!(parts[0], "late night");
        assert_eq!(parts[5], "dawn");
        assert_eq!(parts[8], "morning");
        assert_eq!(parts[12], "midday");
        assert_eq!(parts[16], "afternoon");
        assert_eq!(parts[19], "evening");
        assert_eq!(parts[23], "night");
    }

    #[test]
    fn weather_words_cover_the_wmo_codes_open_meteo_sends() {
        for code in [0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67] {
            assert!(weather_words(code).is_some(), "{code}");
        }
        for code in [71, 73, 75, 77, 80, 81, 82, 85, 86, 95, 96, 99] {
            assert!(weather_words(code).is_some(), "{code}");
        }
        assert_eq!(weather_words(4), None);
    }

    #[test]
    fn temperatures_run_from_bitter_to_scorching() {
        assert_eq!(temperature_words(-20.0), "bitterly cold");
        assert_eq!(temperature_words(0.0), "freezing");
        assert_eq!(temperature_words(12.0), "cool");
        assert_eq!(temperature_words(40.0), "scorching");
    }
}
