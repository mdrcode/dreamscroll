use chrono::{DateTime, Datelike, Months, Utc};

// TODO should we just use chrono_humanize crate instead?

pub fn humanize_datetime(datetime: DateTime<Utc>) -> String {
    let now = Utc::now();

    if datetime > now {
        return format_calendar_date(datetime, now);
    }

    let duration = now - datetime;

    if duration.num_minutes() < 1 {
        return "just now".to_string();
    }

    if duration.num_hours() < 1 {
        let minutes = duration.num_minutes();
        return if minutes == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{minutes} minutes ago")
        };
    }

    if duration.num_days() < 1 {
        let hours = duration.num_hours();
        return if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{hours} hours ago")
        };
    }

    let days = duration.num_days();
    if days == 1 {
        return "yesterday".to_string();
    }

    if days < 7 {
        return format!("{days} days ago");
    }

    format_calendar_date(datetime, now)
}

fn format_calendar_date(created_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let day = created_at.day();
    let suffix = day_ordinal_suffix(day);

    // Simple and arbitrary logic:
    // If same year, don't include year
    // If last year but within the last three months, don't include year
    // Otherwise, include the year

    let three_months_ago = now.date_naive().checked_sub_months(Months::new(3));
    let include_year = match created_at.year().cmp(&now.year()) {
        std::cmp::Ordering::Equal => false,
        std::cmp::Ordering::Less if created_at.year() == now.year() - 1 => match three_months_ago {
            Some(threshold_date) => created_at.date_naive() < threshold_date,
            None => true,
        },
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => true,
    };

    if !include_year {
        format!("{} {day}{suffix}", created_at.format("%B"))
    } else {
        format!("{} {day}, {}", created_at.format("%B"), created_at.year())
    }
}

fn day_ordinal_suffix(day: u32) -> &'static str {
    match day % 100 {
        11..=13 => "th",
        _ => match day % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    }
}
