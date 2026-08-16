use chrono::{DateTime, Datelike, NaiveTime, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPresetOption {
    pub key: String,
    pub label: String,
    pub timezone: String,
    pub start: String,
    pub end: String,
    pub weekdays: Vec<u32>,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct TradingSession {
    pub preset: String,
    pub label: String,
    pub timezone_name: String,
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub weekdays: Vec<u32>,
    pub description: String,
    pub all_day: bool,
}

impl TradingSession {
    pub fn is_open_at(&self, moment: Option<DateTime<Utc>>) -> bool {
        let utc_now = moment.unwrap_or_else(Utc::now);
        let tz: Tz = match Tz::from_str(&self.timezone_name) {
            Ok(t) => t,
            Err(_) => return true,
        };
        let local = utc_now.with_timezone(&tz);
        let weekday_num = local.weekday().num_days_from_monday();

        if self.all_day || self.start == self.end {
            return self.weekdays.contains(&weekday_num);
        }

        let minute_now = local.hour() * 60 + local.minute();
        let start_min = self.start.hour() * 60 + self.start.minute();
        let end_min = self.end.hour() * 60 + self.end.minute();

        if start_min < end_min {
            self.weekdays.contains(&weekday_num) && minute_now >= start_min && minute_now < end_min
        } else {
            let prev_weekday = (weekday_num + 6) % 7;
            (self.weekdays.contains(&weekday_num) && minute_now >= start_min)
                || (self.weekdays.contains(&prev_weekday) && minute_now < end_min)
        }
    }

    pub fn as_dict(&self, moment: Option<DateTime<Utc>>) -> serde_json::Value {
        serde_json::json!({
            "preset": self.preset,
            "label": self.label,
            "timezone": self.timezone_name,
            "start": format!("{:02}:{:02}", self.start.hour(), self.start.minute()),
            "end": format!("{:02}:{:02}", self.end.hour(), self.end.minute()),
            "weekdays": self.weekdays,
            "description": self.description,
            "active": self.is_open_at(moment),
        })
    }
}

pub fn build_trading_session(
    preset: &str,
    timezone_name: &str,
    start: &str,
    end: &str,
    weekdays: Option<&[u32]>,
) -> TradingSession {
    let p = preset.trim().to_lowercase();
    match p.as_str() {
        "us_regular" => TradingSession {
            preset: "us_regular".to_string(),
            label: "美股常规盘".to_string(),
            timezone_name: "America/New_York".to_string(),
            start: NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
            end: NaiveTime::from_hms_opt(16, 0, 0).unwrap(),
            weekdays: vec![0, 1, 2, 3, 4],
            description: "周一至周五，美东时间 09:30-16:00".to_string(),
            all_day: false,
        },
        "us_open" => TradingSession {
            preset: "us_open".to_string(),
            label: "美股开盘窗口".to_string(),
            timezone_name: "America/New_York".to_string(),
            start: NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
            end: NaiveTime::from_hms_opt(11, 30, 0).unwrap(),
            weekdays: vec![0, 1, 2, 3, 4],
            description: "周一至周五，美东时间开盘后两小时".to_string(),
            all_day: false,
        },
        "london" => TradingSession {
            preset: "london".to_string(),
            label: "伦敦时段".to_string(),
            timezone_name: "Europe/London".to_string(),
            start: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(16, 30, 0).unwrap(),
            weekdays: vec![0, 1, 2, 3, 4],
            description: "周一至周五，伦敦当地时间 08:00-16:30".to_string(),
            all_day: false,
        },
        "asia" => TradingSession {
            preset: "asia".to_string(),
            label: "亚洲时段".to_string(),
            timezone_name: "Asia/Shanghai".to_string(),
            start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(16, 0, 0).unwrap(),
            weekdays: vec![0, 1, 2, 3, 4],
            description: "周一至周五，北京时间 09:00-16:00".to_string(),
            all_day: false,
        },
        "custom" => {
            let start_t = NaiveTime::parse_from_str(start, "%H:%M")
                .unwrap_or_else(|_| NaiveTime::from_hms_opt(0, 0, 0).unwrap());
            let end_t = NaiveTime::parse_from_str(end, "%H:%M")
                .unwrap_or_else(|_| NaiveTime::from_hms_opt(0, 0, 0).unwrap());
            let wds = weekdays.map(|w| w.to_vec()).unwrap_or_else(|| vec![0, 1, 2, 3, 4, 5, 6]);

            TradingSession {
                preset: "custom".to_string(),
                label: "自定义时段".to_string(),
                timezone_name: timezone_name.to_string(),
                start: start_t,
                end: end_t,
                weekdays: wds,
                description: "按自定义时区、时间和星期运行".to_string(),
                all_day: start_t == end_t,
            }
        }
        _ => TradingSession {
            preset: "always".to_string(),
            label: "全天候".to_string(),
            timezone_name: "UTC".to_string(),
            start: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            weekdays: vec![0, 1, 2, 3, 4, 5, 6],
            description: "全天运行，适合 7×24 小时市场".to_string(),
            all_day: true,
        },
    }
}

pub fn session_preset_options() -> Vec<SessionPresetOption> {
    vec![
        SessionPresetOption {
            key: "always".to_string(),
            label: "全天候".to_string(),
            timezone: "UTC".to_string(),
            start: "00:00".to_string(),
            end: "00:00".to_string(),
            weekdays: vec![0, 1, 2, 3, 4, 5, 6],
            description: "全天运行，适合 7×24 小时市场".to_string(),
        },
        SessionPresetOption {
            key: "us_regular".to_string(),
            label: "美股常规盘".to_string(),
            timezone: "America/New_York".to_string(),
            start: "09:30".to_string(),
            end: "16:00".to_string(),
            weekdays: vec![0, 1, 2, 3, 4],
            description: "周一至周五，美东时间 09:30-16:00".to_string(),
        },
        SessionPresetOption {
            key: "us_open".to_string(),
            label: "美股开盘窗口".to_string(),
            timezone: "America/New_York".to_string(),
            start: "09:30".to_string(),
            end: "11:30".to_string(),
            weekdays: vec![0, 1, 2, 3, 4],
            description: "周一至周五，美东时间开盘后两小时".to_string(),
        },
        SessionPresetOption {
            key: "london".to_string(),
            label: "伦敦时段".to_string(),
            timezone: "Europe/London".to_string(),
            start: "08:00".to_string(),
            end: "16:30".to_string(),
            weekdays: vec![0, 1, 2, 3, 4],
            description: "周一至周五，伦敦当地时间 08:00-16:30".to_string(),
        },
        SessionPresetOption {
            key: "asia".to_string(),
            label: "亚洲时段".to_string(),
            timezone: "Asia/Shanghai".to_string(),
            start: "09:00".to_string(),
            end: "16:00".to_string(),
            weekdays: vec![0, 1, 2, 3, 4],
            description: "周一至周五，北京时间 09:00-16:00".to_string(),
        },
    ]
}
