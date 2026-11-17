use super::{Host, HostGroup};
use crate::StoreError;
use std::collections::HashSet;
use uuid::Uuid;

/// Groups plus the ordered host list and the rules for rearranging them.
///
/// Display order is the order of `hosts`: ungrouped first, then each group with
/// its hosts in array order. A host pointing at a missing group is ungrouped.
#[derive(Debug, Clone, PartialEq)]
pub struct HostLayout {
    groups: Vec<HostGroup>,
    hosts: Vec<Host>,
}

impl HostLayout {
    pub fn new(groups: Vec<HostGroup>, hosts: Vec<Host>) -> Self {
        let mut layout = Self { groups, hosts };
        layout.normalize();
        layout
    }

    pub fn groups(&self) -> &[HostGroup] {
        &self.groups
    }

    pub fn hosts(&self) -> &[Host] {
        &self.hosts
    }

    pub fn ungrouped(&self) -> Vec<&Host> {
        self.hosts.iter().filter(|h| h.group_id.is_none()).collect()
    }

    pub fn hosts_in(&self, group: Uuid) -> Vec<&Host> {
        self.hosts
            .iter()
            .filter(|h| h.group_id == Some(group))
            .collect()
    }

    pub fn group(&self, id: Option<Uuid>) -> Option<&HostGroup> {
        let id = id?;
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn add_group(&mut self, base: &str) -> HostGroup {
        let mut name = base.to_string();
        let mut suffix = 2;
        while self.groups.iter().any(|g| g.name == name) {
            name = format!("{base} {suffix}");
            suffix += 1;
        }
        let group = HostGroup::new(name);
        self.groups.push(group.clone());
        group
    }

    pub fn rename_group(&mut self, id: Uuid, name: &str) -> Result<(), StoreError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(StoreError("A group name is required.".into()));
        }
        if trimmed.chars().count() > HostGroup::MAX_NAME_LENGTH {
            return Err(StoreError(format!(
                "Group names can be at most {} characters.",
                HostGroup::MAX_NAME_LENGTH
            )));
        }
        if let Some(group) = self.groups.iter_mut().find(|g| g.id == id) {
            group.name = trimmed.to_string();
        }
        Ok(())
    }

    /// Removes the group; its hosts become ungrouped and keep their order.
    pub fn delete_group(&mut self, id: Uuid) {
        self.groups.retain(|g| g.id != id);
        self.normalize();
    }

    /// Moves a group before `before`, or to the end when `before` is `None`.
    pub fn move_group(&mut self, id: Uuid, before: Option<Uuid>) {
        if Some(id) == before {
            return;
        }
        let Some(index) = self.groups.iter().position(|g| g.id == id) else {
            return;
        };
        let group = self.groups.remove(index);
        match before.and_then(|b| self.groups.iter().position(|g| g.id == b)) {
            Some(target) => self.groups.insert(target, group),
            None => self.groups.push(group),
        }
    }

    /// Puts a host in `to_group` (`None` = ungrouped). With `before` it lands
    /// just before that host (if it is in the destination); otherwise after the
    /// last host of that section.
    pub fn move_host(&mut self, id: Uuid, to_group: Option<Uuid>, before: Option<Uuid>) {
        if Some(id) == before {
            return;
        }
        let Some(index) = self.hosts.iter().position(|h| h.id == id) else {
            return;
        };
        let to_group = to_group.filter(|g| self.groups.iter().any(|x| x.id == *g));
        let mut host = self.hosts.remove(index);
        host.group_id = to_group;
        if let Some(target) = before
            .and_then(|b| self.hosts.iter().position(|h| h.id == b))
            .filter(|&i| self.hosts[i].group_id == to_group)
        {
            self.hosts.insert(target, host);
        } else if let Some(last) = self.hosts.iter().rposition(|h| h.group_id == to_group) {
            self.hosts.insert(last + 1, host);
        } else {
            self.hosts.push(host);
        }
    }

    fn normalize(&mut self) {
        let known: HashSet<Uuid> = self.groups.iter().map(|g| g.id).collect();
        for host in &mut self.hosts {
            if host.group_id.is_some_and(|g| !known.contains(&g)) {
                host.group_id = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(name: &str, group: Option<&HostGroup>) -> Host {
        let mut h = Host::new(name, format!("{}.example", name.to_lowercase()));
        h.group_id = group.map(|g| g.id);
        h
    }

    fn names(hosts: &[&Host]) -> Vec<String> {
        hosts.iter().map(|h| h.name.clone()).collect()
    }

    fn all(hosts: &[Host]) -> Vec<String> {
        hosts.iter().map(|h| h.name.clone()).collect()
    }

    #[test]
    fn ungrouped_hosts_and_each_group_keep_their_own_order() {
        let prod = HostGroup::new("Prod");
        let lab = HostGroup::new("Lab");
        let layout = HostLayout::new(
            vec![prod.clone(), lab.clone()],
            vec![
                host("A", None),
                host("P1", Some(&prod)),
                host("L1", Some(&lab)),
                host("B", None),
                host("P2", Some(&prod)),
            ],
        );
        assert_eq!(names(&layout.ungrouped()), ["A", "B"]);
        assert_eq!(names(&layout.hosts_in(prod.id)), ["P1", "P2"]);
        assert_eq!(names(&layout.hosts_in(lab.id)), ["L1"]);
    }

    #[test]
    fn a_host_pointing_at_a_missing_group_is_ungrouped() {
        let gone = HostGroup::new("Gone");
        let layout = HostLayout::new(vec![], vec![host("A", Some(&gone))]);
        assert_eq!(names(&layout.ungrouped()), ["A"]);
        assert_eq!(layout.hosts()[0].group_id, None);
    }

    #[test]
    fn dropping_on_a_group_appends_the_host_to_it() {
        let prod = HostGroup::new("Prod");
        let a = host("A", None);
        let mut layout = HostLayout::new(
            vec![prod.clone()],
            vec![a.clone(), host("P1", Some(&prod)), host("B", None)],
        );
        layout.move_host(a.id, Some(prod.id), None);
        assert_eq!(names(&layout.ungrouped()), ["B"]);
        assert_eq!(names(&layout.hosts_in(prod.id)), ["P1", "A"]);
    }

    #[test]
    fn dropping_on_a_host_takes_its_group_and_place() {
        let prod = HostGroup::new("Prod");
        let a = host("A", None);
        let p1 = host("P1", Some(&prod));
        let p2 = host("P2", Some(&prod));
        let mut layout =
            HostLayout::new(vec![prod.clone()], vec![a.clone(), p1.clone(), p2.clone()]);
        layout.move_host(a.id, Some(prod.id), Some(p2.id));
        assert_eq!(names(&layout.hosts_in(prod.id)), ["P1", "A", "P2"]);
        assert!(layout.ungrouped().is_empty());

        let b = host("B", None);
        let c = host("C", None);
        let mut layout =
            HostLayout::new(vec![prod.clone()], vec![b.clone(), c.clone(), p1.clone()]);
        layout.move_host(c.id, None, Some(b.id));
        assert_eq!(names(&layout.ungrouped()), ["C", "B"]);
        layout.move_host(p1.id, None, None);
        assert_eq!(names(&layout.ungrouped()), ["C", "B", "P1"]);
        assert!(layout.hosts_in(prod.id).is_empty());
    }

    #[test]
    fn moving_a_host_onto_itself_changes_nothing() {
        let a = host("A", None);
        let b = host("B", None);
        let mut layout = HostLayout::new(vec![], vec![a.clone(), b]);
        layout.move_host(a.id, None, Some(a.id));
        assert_eq!(all(layout.hosts()), ["A", "B"]);
    }

    #[test]
    fn groups_reorder_and_deleting_one_keeps_its_hosts_ungrouped() {
        let one = HostGroup::new("One");
        let two = HostGroup::new("Two");
        let three = HostGroup::new("Three");
        let mut layout = HostLayout::new(
            vec![one.clone(), two.clone(), three.clone()],
            vec![
                host("A", None),
                host("T1", Some(&two)),
                host("T2", Some(&two)),
            ],
        );
        layout.move_group(three.id, Some(one.id));
        let order = |l: &HostLayout| {
            l.groups()
                .iter()
                .map(|g| g.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&layout), ["Three", "One", "Two"]);
        layout.move_group(three.id, None);
        assert_eq!(order(&layout), ["One", "Two", "Three"]);
        layout.delete_group(two.id);
        assert_eq!(order(&layout), ["One", "Three"]);
        assert_eq!(names(&layout.ungrouped()), ["A", "T1", "T2"]);
    }

    #[test]
    fn new_groups_get_unique_names_and_renames_are_trimmed() {
        let mut layout = HostLayout::new(vec![], vec![]);
        let first = layout.add_group("New Group");
        let second = layout.add_group("New Group");
        assert_eq!(first.name, "New Group");
        assert_eq!(second.name, "New Group 2");
        layout.rename_group(first.id, "  Production  ").unwrap();
        assert_eq!(layout.groups()[0].name, "Production");
        assert!(layout.rename_group(first.id, "   ").is_err());
        assert!(layout.rename_group(first.id, &"x".repeat(129)).is_err());
        assert_eq!(layout.groups()[0].name, "Production");
    }
}
