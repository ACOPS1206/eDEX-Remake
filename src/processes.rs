use crate::metrics::Process;
use std::{cell::RefCell, cmp::Ordering, rc::Rc};
use vte4::gtk::{self, gio, glib, pango, prelude::*};

#[derive(Clone, Default)]
pub struct Inspector {
    latest: Rc<RefCell<Vec<Process>>>,
    visible: Rc<glib::WeakRef<gtk::Overlay>>,
    model: Rc<glib::WeakRef<gio::ListStore>>,
}

struct Column {
    name: &'static str,
    format: fn(&Process) -> String,
    order: fn(&Process, &Process) -> Ordering,
}

const COLUMNS: [Column; 8] = [
    Column {
        name: "PID",
        format: |p| p.pid.to_string(),
        order: |a, b| a.pid.cmp(&b.pid),
    },
    Column {
        name: "Name",
        format: |p| p.name.clone(),
        order: |a, b| a.name.cmp(&b.name),
    },
    Column {
        name: "User",
        format: |p| p.user.clone(),
        order: |a, b| a.user.cmp(&b.user),
    },
    Column {
        name: "CPU",
        format: |p| format!("{:.1}%", p.cpu),
        order: |a, b| a.cpu.total_cmp(&b.cpu),
    },
    Column {
        name: "Memory",
        format: |p| format!("{:.1}%", p.memory_percent),
        order: |a, b| a.memory.cmp(&b.memory),
    },
    Column {
        name: "State",
        format: |p| p.state.clone(),
        order: |a, b| a.state.cmp(&b.state),
    },
    Column {
        name: "Started",
        format: |p| {
            glib::DateTime::from_unix_local(p.started as i64)
                .and_then(|d| d.format("%Y-%m-%d %H:%M:%S"))
                .map(|d| d.to_string())
                .unwrap_or_default()
        },
        order: |a, b| a.started.cmp(&b.started),
    },
    Column {
        name: "Runtime",
        format: |p| {
            format!(
                "{:02}:{:02}:{:02}:{:02}",
                p.runtime / 86400,
                p.runtime / 3600 % 24,
                p.runtime / 60 % 60,
                p.runtime % 60
            )
        },
        order: |a, b| a.runtime.cmp(&b.runtime),
    },
];

impl Inspector {
    pub fn update(&self, processes: Vec<Process>) {
        if let Some(model) = self.model.upgrade() {
            populate(&model, &processes);
        }
        self.latest.replace(processes);
    }

    pub fn open(&self, parent: &gtk::ApplicationWindow) {
        if self.visible.upgrade().is_some() {
            return;
        }
        let model = gio::ListStore::new::<glib::BoxedAnyObject>();
        populate(&model, &self.latest.borrow());
        let view = gtk::ColumnView::new(None::<gtk::NoSelection>);
        view.add_css_class("process-inspector");
        view.set_show_column_separators(true);
        view.set_show_row_separators(true);
        for column in &COLUMNS {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(|_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                let label = gtk::Label::builder()
                    .xalign(0.0)
                    .ellipsize(pango::EllipsizeMode::End)
                    .build();
                item.set_child(Some(&label));
            });
            factory.connect_bind(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                let process = item.item().and_downcast::<glib::BoxedAnyObject>().unwrap();
                let label = item.child().and_downcast::<gtk::Label>().unwrap();
                label.set_text(&(column.format)(&process.borrow::<Process>()));
            });
            let heading = gtk::ColumnViewColumn::new(Some(column.name), Some(factory));
            let sorter = gtk::CustomSorter::new(move |a, b| {
                let a = a
                    .downcast_ref::<glib::BoxedAnyObject>()
                    .unwrap()
                    .borrow::<Process>();
                let b = b
                    .downcast_ref::<glib::BoxedAnyObject>()
                    .unwrap()
                    .borrow::<Process>();
                (column.order)(&a, &b).into()
            });
            heading.set_sorter(Some(&sorter));
            heading.set_expand(matches!(column.name, "Name" | "Started"));
            heading.set_resizable(true);
            view.append_column(&heading);
        }
        if let Some(heading) = view
            .columns()
            .item(3)
            .and_downcast::<gtk::ColumnViewColumn>()
        {
            view.sort_by_column(Some(&heading), gtk::SortType::Descending);
        }
        let sorted = gtk::SortListModel::new(Some(model.clone()), view.sorter());
        view.set_model(Some(&gtk::NoSelection::new(Some(sorted))));
        let scroll = gtk::ScrolledWindow::builder().child(&view).build();
        let dialog = crate::settings::dialog(parent, "Active Processes", 1040, 560, &scroll);
        self.model.set(Some(&model));
        self.visible.set(Some(&dialog.layer));
        dialog.present();
    }
}

fn populate(model: &gio::ListStore, processes: &[Process]) {
    let rows: Vec<_> = processes
        .iter()
        .cloned()
        .map(glib::BoxedAnyObject::new)
        .collect();
    model.splice(0, model.n_items(), &rows);
}
