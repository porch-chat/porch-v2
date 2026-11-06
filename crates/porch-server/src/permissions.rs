//! Pure bounded community aggregate and Discord-style permission calculation.
//! Only CommunityService persists this aggregate, under a community row lock.
use crate::identity::ApiError;
use porch_protocol::{permissions::*, *};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

type Result<T> = std::result::Result<T, ApiError>;
fn forbidden<T>() -> Result<T> {
    Err(ApiError(ErrorCode::Forbidden))
}
fn invalid<T>() -> Result<T> {
    Err(ApiError(ErrorCode::InvalidRequest))
}
fn missing<T>() -> Result<T> {
    Err(ApiError(ErrorCode::NotFound))
}

pub fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 100 && !name.chars().any(char::is_control)
}
fn name(s: &str) -> Result<()> {
    if valid_name(s) { Ok(()) } else { invalid() }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Community {
    pub schema_version: u16,
    pub summary: CommunitySummary,
    pub roles: Vec<Role>,
    pub members: Vec<Member>,
    pub categories: Vec<Category>,
    pub channels: Vec<Channel>,
    pub bans: Vec<i64>,
    next_id: i64,
}

pub fn implicit(mut bits: u64) -> u64 {
    bits &= ALL;
    if bits & VIEW_CHANNEL == 0 {
        return 0;
    }
    if bits & SEND_MESSAGES == 0 {
        bits &= !SEND_DEPENDENT;
    }
    if bits & CONNECT == 0 {
        bits &= !CONNECT_DEPENDENT;
    }
    bits
}
fn apply(bits: u64, deny: u64, allow: u64) -> u64 {
    (bits & !deny) | allow
}

impl Community {
    pub fn new(id: i64, owner_id: i64, title: String, official: bool) -> Result<Self> {
        name(&title)?;
        Ok(Self {
            schema_version: 1,
            summary: CommunitySummary {
                id,
                owner_id,
                name: title,
                official,
            },
            roles: vec![Role {
                id: 0,
                name: "@everyone".into(),
                permissions: if official {
                    VIEW_CHANNEL | READ_HISTORY | CONNECT
                } else {
                    DEFAULT
                },
                position: 0,
            }],
            members: vec![Member {
                user_id: owner_id,
                role_ids: vec![],
            }],
            categories: vec![],
            channels: vec![],
            bans: vec![],
            next_id: 1,
        })
    }
    fn allocate(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn member(&self, user: i64) -> Option<&Member> {
        self.members.iter().find(|m| m.user_id == user)
    }
    pub fn base(&self, user: i64) -> u64 {
        let Some(member) = self.member(user) else {
            return 0;
        };
        if user == self.summary.owner_id {
            return ALL;
        }
        let bits = self
            .roles
            .iter()
            .filter(|r| r.id == 0 || member.role_ids.contains(&r.id))
            .fold(0, |bits, r| bits | r.permissions);
        if bits & ADMINISTRATOR != 0 { ALL } else { bits }
    }
    pub fn effective(&self, user: i64, overwrites: &[PermissionOverwrite]) -> u64 {
        let base = self.base(user);
        if base & ADMINISTRATOR != 0 {
            return ALL;
        }
        let Some(member) = self.member(user) else {
            return 0;
        };
        let mut result = base;
        if let Some(o) = overwrites
            .iter()
            .find(|o| o.target == OverwriteTarget::Everyone)
        {
            result = apply(result, o.deny, o.allow);
        }
        let (deny, allow) = overwrites
            .iter()
            .filter(
                |o| matches!(o.target,OverwriteTarget::Role(id) if member.role_ids.contains(&id)),
            )
            .fold((0, 0), |(d, a), o| (d | o.deny, a | o.allow));
        result = apply(result, deny, allow);
        if let Some(o) = overwrites
            .iter()
            .find(|o| o.target == OverwriteTarget::Member(user))
        {
            result = apply(result, o.deny, o.allow);
        }
        implicit(result)
    }
    pub fn channel_bits(&self, user: i64, c: &Channel) -> u64 {
        let overwrites = if c.synced {
            self.categories
                .iter()
                .find(|cat| Some(cat.id) == c.category_id)
                .map_or(&[][..], |cat| cat.overwrites.as_slice())
        } else {
            &c.overwrites
        };
        self.effective(user, overwrites)
    }
    fn highest(&self, user: i64) -> usize {
        self.member(user).map_or(0, |m| {
            self.roles
                .iter()
                .filter(|r| m.role_ids.contains(&r.id))
                .map(|r| r.position)
                .max()
                .unwrap_or(0)
        })
    }
    fn require(&self, user: i64, bits: u64) -> Result<()> {
        if self.member(user).is_some() && self.base(user) & bits == bits {
            Ok(())
        } else {
            forbidden()
        }
    }
    fn above_role(&self, actor: i64, role: &Role) -> Result<()> {
        if actor == self.summary.owner_id || self.highest(actor) > role.position {
            Ok(())
        } else {
            forbidden()
        }
    }
    fn above_member(&self, actor: i64, target: i64) -> Result<()> {
        if target == self.summary.owner_id {
            return forbidden();
        }
        if actor == self.summary.owner_id || self.highest(actor) > self.highest(target) {
            Ok(())
        } else {
            forbidden()
        }
    }
    fn grant(&self, actor: i64, bits: u64) -> Result<()> {
        if bits & !ALL != 0 {
            return invalid();
        }
        if bits & !self.base(actor) != 0 {
            forbidden()
        } else {
            Ok(())
        }
    }
    fn category(&self, id: i64) -> Result<&Category> {
        self.categories
            .iter()
            .find(|c| c.id == id)
            .ok_or(ApiError(ErrorCode::NotFound))
    }
    fn channel(&self, id: i64) -> Result<&Channel> {
        self.channels
            .iter()
            .find(|c| c.id == id)
            .ok_or(ApiError(ErrorCode::NotFound))
    }
    fn manage_channel(&self, actor: i64, id: i64) -> Result<()> {
        let c = self.channel(id)?;
        let bits = self.channel_bits(actor, c);
        if bits & (VIEW_CHANNEL | MANAGE_CHANNELS) == (VIEW_CHANNEL | MANAGE_CHANNELS) {
            Ok(())
        } else {
            forbidden()
        }
    }
    fn manage_category(&self, actor: i64, id: Option<i64>) -> Result<()> {
        let bits = match id {
            Some(id) => self.effective(actor, &self.category(id)?.overwrites),
            None => self.base(actor),
        };
        if bits & (VIEW_CHANNEL | MANAGE_CHANNELS) == (VIEW_CHANNEL | MANAGE_CHANNELS) {
            Ok(())
        } else {
            forbidden()
        }
    }
    pub fn snapshot(&self, user: i64) -> Result<CommunitySnapshot> {
        if self.member(user).is_none() {
            return missing();
        }
        // Construct a new response, never serialize the internal aggregate.
        let channels: Vec<_> = self
            .channels
            .iter()
            .filter(|c| self.channel_bits(user, c) & VIEW_CHANNEL != 0)
            .cloned()
            .collect();
        let categories = self
            .categories
            .iter()
            .filter(|cat| {
                channels.iter().any(|c| c.category_id == Some(cat.id))
                    || (!self.channels.iter().any(|c| c.category_id == Some(cat.id))
                        && self.effective(user, &cat.overwrites) & VIEW_CHANNEL != 0)
            })
            .cloned()
            .collect();
        Ok(CommunitySnapshot {
            community: self.summary.clone(),
            roles: self.roles.clone(),
            members: self.members.clone(),
            categories,
            channels,
            bans: if self.base(user) & BAN_MEMBERS != 0 {
                self.bans.clone()
            } else {
                vec![]
            },
            permissions: self.base(user),
        })
    }
    pub fn join(&mut self, user: i64) -> Result<()> {
        if self.bans.contains(&user) {
            return forbidden();
        }
        if self.member(user).is_some() {
            return Err(ApiError(ErrorCode::Conflict));
        }
        if self.members.len() >= 200 {
            return forbidden();
        }
        self.members.push(Member {
            user_id: user,
            role_ids: vec![],
        });
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.members.len() > 200
            || self.roles.len() > 128
            || self.channels.len() > 512
            || self.categories.len() > 128
            || self.bans.len() > 200
        {
            return invalid();
        }
        if self.member(self.summary.owner_id).is_none()
            || self.bans.contains(&self.summary.owner_id)
        {
            return invalid();
        }
        let unique = |ids: Vec<i64>| ids.len() == ids.iter().collect::<HashSet<_>>().len();
        if !unique(self.members.iter().map(|m| m.user_id).collect()) || !unique(self.bans.clone()) {
            return invalid();
        }
        let mut all_ids = vec![0];
        all_ids.extend(self.roles.iter().filter(|r| r.id != 0).map(|r| r.id));
        all_ids.extend(self.channels.iter().map(|c| c.id));
        all_ids.extend(self.categories.iter().map(|c| c.id));
        if !unique(all_ids.clone()) || all_ids.iter().any(|id| *id < 0 || *id >= self.next_id) {
            return invalid();
        }
        let mut positions: Vec<_> = self.roles.iter().map(|r| r.position).collect();
        positions.sort();
        if positions != (0..self.roles.len()).collect::<Vec<_>>() {
            return invalid();
        }
        let everyone: Vec<_> = self.roles.iter().filter(|r| r.id == 0).collect();
        if everyone.len() != 1 || everyone[0].position != 0 || everyone[0].name != "@everyone" {
            return invalid();
        }
        for r in &self.roles {
            name(&r.name)?;
            if r.permissions & !ALL != 0 {
                return invalid();
            }
        }
        for m in &self.members {
            if self.bans.contains(&m.user_id)
                || !unique(m.role_ids.clone())
                || m.role_ids
                    .iter()
                    .any(|id| *id == 0 || !self.roles.iter().any(|r| r.id == *id))
            {
                return invalid();
            }
        }
        let mut cat_positions: Vec<_> = self.categories.iter().map(|c| c.position).collect();
        cat_positions.sort();
        if cat_positions != (0..self.categories.len()).collect::<Vec<_>>() {
            return invalid();
        }
        for cat in &self.categories {
            name(&cat.name)?;
            self.valid_overwrites(&cat.overwrites)?;
        }
        for c in &self.channels {
            name(&c.name)?;
            self.valid_overwrites(&c.overwrites)?;
            if c.synced && (c.category_id.is_none() || !c.overwrites.is_empty()) {
                return invalid();
            }
            if c.category_id.is_some_and(|id| self.category(id).is_err()) {
                return invalid();
            }
            let mut siblings: Vec<_> = self
                .channels
                .iter()
                .filter(|other| other.category_id == c.category_id)
                .map(|other| other.position)
                .collect();
            siblings.sort();
            if siblings != (0..siblings.len()).collect::<Vec<_>>() {
                return invalid();
            }
        }
        Ok(())
    }
    fn valid_overwrites(&self, os: &[PermissionOverwrite]) -> Result<()> {
        if os.len() > 328 {
            return invalid();
        }
        for (i, o) in os.iter().enumerate() {
            if o.allow & o.deny != 0
                || (o.allow | o.deny) & (!ALL | ADMINISTRATOR) != 0
                || os[..i].iter().any(|other| other.target == o.target)
            {
                return invalid();
            }
            match o.target {
                OverwriteTarget::Role(id) if id == 0 || !self.roles.iter().any(|r| r.id == id) => {
                    return invalid();
                }
                OverwriteTarget::Member(id) if self.member(id).is_none() => return invalid(),
                _ => {}
            }
        }
        Ok(())
    }
    fn reorder_roles(&mut self, id: i64, position: usize) -> Result<()> {
        if position == 0 || position >= self.roles.len() {
            return invalid();
        }
        self.roles.sort_by_key(|r| r.position);
        let i = self
            .roles
            .iter()
            .position(|r| r.id == id)
            .ok_or(ApiError(ErrorCode::NotFound))?;
        let role = self.roles.remove(i);
        self.roles.insert(position, role);
        for (p, r) in self.roles.iter_mut().enumerate() {
            r.position = p;
        }
        Ok(())
    }
    fn remove_member(&mut self, user: i64) {
        self.members.retain(|m| m.user_id != user);
        for c in &mut self.channels {
            c.overwrites
                .retain(|o| o.target != OverwriteTarget::Member(user));
        }
        for c in &mut self.categories {
            c.overwrites
                .retain(|o| o.target != OverwriteTarget::Member(user));
        }
    }
    fn normalize_channels(&mut self) {
        self.channels
            .sort_by_key(|c| (c.category_id, c.position, c.id));
        let mut group = None;
        let mut pos = 0;
        for c in &mut self.channels {
            if group != Some(c.category_id) {
                group = Some(c.category_id);
                pos = 0;
            }
            c.position = pos;
            pos += 1;
        }
    }
    fn move_channel(&mut self, id: i64, category: Option<i64>, position: usize) -> Result<()> {
        if let Some(cat) = category {
            self.category(cat)?;
        }
        let index = self
            .channels
            .iter()
            .position(|c| c.id == id)
            .ok_or(ApiError(ErrorCode::NotFound))?;
        let mut channel = self.channels.remove(index);
        self.normalize_channels();
        let len = self
            .channels
            .iter()
            .filter(|c| c.category_id == category)
            .count();
        if position > len {
            return invalid();
        }
        for c in self
            .channels
            .iter_mut()
            .filter(|c| c.category_id == category && c.position >= position)
        {
            c.position += 1;
        }
        channel.category_id = category;
        channel.position = position;
        self.channels.push(channel);
        self.normalize_channels();
        Ok(())
    }
    /// Apply to a cloned aggregate; callers discard it on any error.
    pub fn mutate(&mut self, actor: i64, cmd: CommunityCommand) -> Result<Option<i64>> {
        if self.member(actor).is_none() {
            return missing();
        }
        let mut output = None;
        match cmd {
            CommunityCommand::Rename { name: title } => {
                self.require(actor, MANAGE_COMMUNITY)?;
                name(&title)?;
                self.summary.name = title;
            }
            CommunityCommand::TransferOwnership { user_id } => {
                if actor != self.summary.owner_id || self.summary.official {
                    return forbidden();
                }
                if self.member(user_id).is_none() {
                    return invalid();
                }
                self.summary.owner_id = user_id;
            }
            CommunityCommand::Delete => {
                if actor != self.summary.owner_id || self.summary.official {
                    return forbidden();
                }
                // Persistence layer handles deletion after this authorization.
            }
            CommunityCommand::CreateRole {
                name: title,
                permissions,
                position,
            } => {
                self.require(actor, MANAGE_ROLES)?;
                self.grant(actor, permissions)?;
                name(&title)?;
                if self.roles.len() >= 128 || position == 0 || position > self.roles.len() {
                    return invalid();
                }
                if actor != self.summary.owner_id && position >= self.highest(actor) {
                    return forbidden();
                }
                let id = self.allocate();
                let end = self.roles.len();
                self.roles.push(Role {
                    id,
                    name: title,
                    permissions,
                    position: end,
                });
                self.reorder_roles(id, position)?;
                output = Some(id);
            }
            CommunityCommand::EditRole {
                id,
                name: title,
                permissions,
                position,
            } => {
                self.require(actor, MANAGE_ROLES)?;
                self.grant(actor, permissions)?;
                name(&title)?;
                let role = self
                    .roles
                    .iter()
                    .find(|r| r.id == id)
                    .ok_or(ApiError(ErrorCode::NotFound))?;
                if id == 0 {
                    if title != "@everyone" || position != 0 {
                        return invalid();
                    }
                } else {
                    self.above_role(actor, role)?;
                    if actor != self.summary.owner_id && position >= self.highest(actor) {
                        return forbidden();
                    }
                    self.reorder_roles(id, position)?;
                }
                let r = self.roles.iter_mut().find(|r| r.id == id).unwrap();
                r.name = title;
                r.permissions = permissions;
            }
            CommunityCommand::DeleteRole { id } => {
                self.require(actor, MANAGE_ROLES)?;
                if id == 0 {
                    return forbidden();
                }
                self.above_role(
                    actor,
                    self.roles
                        .iter()
                        .find(|r| r.id == id)
                        .ok_or(ApiError(ErrorCode::NotFound))?,
                )?;
                self.roles.retain(|r| r.id != id);
                self.roles.sort_by_key(|r| r.position);
                for (p, r) in self.roles.iter_mut().enumerate() {
                    r.position = p;
                }
                for m in &mut self.members {
                    m.role_ids.retain(|r| *r != id);
                }
                for c in &mut self.categories {
                    c.overwrites
                        .retain(|o| o.target != OverwriteTarget::Role(id));
                }
                for c in &mut self.channels {
                    c.overwrites
                        .retain(|o| o.target != OverwriteTarget::Role(id));
                }
            }
            CommunityCommand::SetMemberRoles { user_id, role_ids } => {
                self.require(actor, MANAGE_ROLES)?;
                self.above_member(actor, user_id)?;
                let m = self.member(user_id).ok_or(ApiError(ErrorCode::NotFound))?;
                if role_ids.len() > 127 {
                    return invalid();
                }
                for id in role_ids.iter().chain(m.role_ids.iter()) {
                    let r = self
                        .roles
                        .iter()
                        .find(|r| r.id == *id)
                        .ok_or(ApiError(ErrorCode::InvalidRequest))?;
                    if *id == 0 {
                        return invalid();
                    }
                    // Even removal of a privileged role requires hierarchy and possessed permissions.
                    if m.role_ids.contains(id) != role_ids.contains(id) {
                        self.above_role(actor, r)?;
                        self.grant(actor, r.permissions)?;
                    }
                }
                self.members
                    .iter_mut()
                    .find(|m| m.user_id == user_id)
                    .unwrap()
                    .role_ids = role_ids;
            }
            CommunityCommand::CreateCategory {
                name: title,
                position,
            } => {
                self.require(actor, MANAGE_CHANNELS)?;
                name(&title)?;
                if self.categories.len() >= 128 || position > self.categories.len() {
                    return invalid();
                }
                for c in self
                    .categories
                    .iter_mut()
                    .filter(|c| c.position >= position)
                {
                    c.position += 1;
                }
                let id = self.allocate();
                self.categories.push(Category {
                    id,
                    name: title,
                    position,
                    overwrites: vec![],
                });
                output = Some(id);
            }
            CommunityCommand::EditCategory {
                id,
                name: title,
                position,
            } => {
                self.manage_category(actor, Some(id))?;
                name(&title)?;
                if position >= self.categories.len() {
                    return invalid();
                }
                self.categories.sort_by_key(|c| c.position);
                let i = self.categories.iter().position(|c| c.id == id).unwrap();
                let mut c = self.categories.remove(i);
                c.name = title;
                self.categories.insert(position, c);
                for (p, c) in self.categories.iter_mut().enumerate() {
                    c.position = p;
                }
            }
            CommunityCommand::DeleteCategory { id } => {
                self.manage_category(actor, Some(id))?;
                if self.channels.iter().any(|c| c.category_id == Some(id)) {
                    return Err(ApiError(ErrorCode::Conflict));
                }
                self.categories.retain(|c| c.id != id);
                self.categories.sort_by_key(|c| c.position);
                for (p, c) in self.categories.iter_mut().enumerate() {
                    c.position = p;
                }
            }
            CommunityCommand::CreateChannel {
                name: title,
                kind,
                category_id,
                position,
                synced,
            } => {
                self.manage_category(actor, category_id)?;
                name(&title)?;
                if self.channels.len() >= 512 || (synced && category_id.is_none()) {
                    return invalid();
                }
                let id = self.allocate();
                self.channels.push(Channel {
                    id,
                    name: title,
                    kind,
                    category_id: None,
                    position: self
                        .channels
                        .iter()
                        .filter(|c| c.category_id.is_none())
                        .count(),
                    synced,
                    overwrites: vec![],
                });
                self.move_channel(id, category_id, position)?;
                output = Some(id);
            }
            CommunityCommand::EditChannel {
                id,
                name: title,
                category_id,
                position,
                synced,
            } => {
                self.manage_channel(actor, id)?;
                self.manage_category(actor, category_id)?;
                name(&title)?;
                if synced && category_id.is_none() {
                    return invalid();
                }
                // Re-sync/move can introduce category allows. May not grant bits actor lacks.
                if let Some(cat) = category_id.filter(|_| synced) {
                    for o in &self.category(cat)?.overwrites {
                        self.grant(actor, o.allow)?;
                    }
                }
                self.move_channel(id, category_id, position)?;
                let c = self.channels.iter_mut().find(|c| c.id == id).unwrap();
                c.name = title;
                c.synced = synced;
                if synced {
                    c.overwrites.clear();
                }
            }
            CommunityCommand::DeleteChannel { id } => {
                self.manage_channel(actor, id)?;
                self.channels.retain(|c| c.id != id);
                self.normalize_channels();
            }
            CommunityCommand::SetOverwrites {
                channel_id,
                category_id,
                overwrites,
            } => {
                self.require(actor, MANAGE_ROLES)?;
                self.valid_overwrites(&overwrites)?;
                let (old, bits) = match (channel_id, category_id) {
                    (Some(id), None) => {
                        self.manage_channel(actor, id)?;
                        let c = self.channel(id)?;
                        if c.synced {
                            return invalid();
                        }
                        (c.overwrites.clone(), self.channel_bits(actor, c))
                    }
                    (None, Some(id)) => {
                        self.manage_category(actor, Some(id))?;
                        let c = self.category(id)?;
                        (c.overwrites.clone(), self.effective(actor, &c.overwrites))
                    }
                    _ => return invalid(),
                };
                for o in overwrites.iter().chain(old.iter()) {
                    let previous = old.iter().find(|p| p.target == o.target);
                    let next = overwrites.iter().find(|p| p.target == o.target);
                    let changed = previous.map_or(0, |p| p.allow | p.deny)
                        ^ next.map_or(0, |p| p.allow | p.deny);
                    // Include changed polarity as well as newly mentioned bits.
                    let changed =
                        changed | (previous.map_or(0, |p| p.allow) ^ next.map_or(0, |p| p.allow));
                    if changed & !bits != 0 {
                        return forbidden();
                    }
                    match o.target {
                        OverwriteTarget::Role(id) => self.above_role(
                            actor,
                            self.roles
                                .iter()
                                .find(|r| r.id == id)
                                .ok_or(ApiError(ErrorCode::InvalidRequest))?,
                        )?,
                        OverwriteTarget::Member(id) if id != actor => {
                            self.above_member(actor, id)?
                        }
                        _ => {}
                    }
                }
                if let Some(id) = channel_id {
                    self.channels
                        .iter_mut()
                        .find(|c| c.id == id)
                        .unwrap()
                        .overwrites = overwrites;
                } else {
                    self.categories
                        .iter_mut()
                        .find(|c| Some(c.id) == category_id)
                        .unwrap()
                        .overwrites = overwrites;
                }
            }
            CommunityCommand::Leave => {
                if actor == self.summary.owner_id {
                    return forbidden();
                }
                self.remove_member(actor);
            }
            CommunityCommand::Kick { user_id } => {
                self.require(actor, KICK_MEMBERS)?;
                self.above_member(actor, user_id)?;
                if self.member(user_id).is_none() {
                    return missing();
                }
                self.remove_member(user_id);
            }
            CommunityCommand::Ban { user_id } => {
                self.require(actor, BAN_MEMBERS)?;
                self.above_member(actor, user_id)?;
                if self.bans.contains(&user_id) {
                    return Err(ApiError(ErrorCode::Conflict));
                }
                if self.bans.len() >= 200 {
                    return invalid();
                }
                self.remove_member(user_id);
                self.bans.push(user_id);
            }
            CommunityCommand::Unban { user_id } => {
                self.require(actor, BAN_MEMBERS)?;
                if !self.bans.contains(&user_id) {
                    return missing();
                }
                self.bans.retain(|id| *id != user_id);
            }
        }
        self.validate()?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    fn fixture() -> Community {
        let mut c = Community::new(1, 10, "test".into(), false).unwrap();
        c.join(20).unwrap();
        c.join(30).unwrap();
        c.roles.push(Role {
            id: 1,
            name: "low".into(),
            permissions: 0,
            position: 1,
        });
        c.roles.push(Role {
            id: 2,
            name: "high".into(),
            permissions: MANAGE_ROLES | MANAGE_CHANNELS | KICK_MEMBERS | BAN_MEMBERS,
            position: 2,
        });
        c.members[1].role_ids = vec![1, 2];
        c.next_id = 3;
        c
    }
    fn ow(target: OverwriteTarget, allow: u64, deny: u64) -> PermissionOverwrite {
        PermissionOverwrite {
            target,
            allow,
            deny,
        }
    }
    #[test]
    fn discord_overwrite_precedence() {
        let c = fixture();
        let os = vec![
            ow(OverwriteTarget::Everyone, 0, SEND_MESSAGES),
            ow(OverwriteTarget::Role(1), SEND_MESSAGES, CONNECT),
            ow(OverwriteTarget::Role(2), CONNECT, SEND_MESSAGES),
        ];
        let bits = c.effective(20, &os);
        assert_ne!(
            bits & SEND_MESSAGES,
            0,
            "aggregate role allows override role denies"
        );
        assert_ne!(bits & CONNECT, 0);
        let mut os = os;
        os.push(ow(OverwriteTarget::Member(20), 0, SEND_MESSAGES | CONNECT));
        let bits = c.effective(20, &os);
        assert_eq!(
            bits & (SEND_MESSAGES
                | ATTACH_FILES
                | MENTION_EVERYONE
                | CONNECT
                | SPEAK
                | STREAM
                | USE_VAD),
            0
        );
    }
    #[test]
    fn owner_admin_bypass_but_nonmember_never() {
        let mut c = fixture();
        let os = vec![ow(OverwriteTarget::Everyone, 0, VIEW_CHANNEL)];
        assert_eq!(c.effective(10, &os), ALL);
        c.roles[1].permissions = ADMINISTRATOR;
        assert_eq!(c.effective(20, &os), ALL);
        assert_eq!(
            c.effective(
                99,
                &[ow(OverwriteTarget::Member(99), ALL & !ADMINISTRATOR, 0)]
            ),
            0
        );
    }
    #[test]
    fn hierarchy_owner_everyone_and_escalation() {
        let c = fixture();
        let deny = |cmd| assert!(c.clone().mutate(20, cmd).is_err());
        deny(CommunityCommand::EditRole {
            id: 2,
            name: "high".into(),
            permissions: ADMINISTRATOR,
            position: 2,
        });
        deny(CommunityCommand::DeleteRole { id: 2 });
        deny(CommunityCommand::DeleteRole { id: 0 });
        deny(CommunityCommand::CreateRole {
            name: "bad".into(),
            permissions: ADMINISTRATOR,
            position: 1,
        });
        deny(CommunityCommand::CreateRole {
            name: "equal".into(),
            permissions: 0,
            position: 2,
        });
        deny(CommunityCommand::SetMemberRoles {
            user_id: 20,
            role_ids: vec![1],
        });
        deny(CommunityCommand::Ban { user_id: 10 });
        deny(CommunityCommand::Kick { user_id: 20 });
        deny(CommunityCommand::EditRole {
            id: 0,
            name: "renamed".into(),
            permissions: DEFAULT,
            position: 0,
        });
        assert!(
            c.clone()
                .mutate(
                    20,
                    CommunityCommand::SetMemberRoles {
                        user_id: 30,
                        role_ids: vec![1]
                    }
                )
                .is_ok()
        );
    }
    #[test]
    fn synced_category_visibility_and_explicit_override() {
        let mut c = fixture();
        let cat = c
            .mutate(
                10,
                CommunityCommand::CreateCategory {
                    name: "hidden".into(),
                    position: 0,
                },
            )
            .unwrap()
            .unwrap();
        let ch = c
            .mutate(
                10,
                CommunityCommand::CreateChannel {
                    name: "secret".into(),
                    kind: ChannelKind::Text,
                    category_id: Some(cat),
                    position: 0,
                    synced: true,
                },
            )
            .unwrap()
            .unwrap();
        c.mutate(
            10,
            CommunityCommand::SetOverwrites {
                channel_id: None,
                category_id: Some(cat),
                overwrites: vec![ow(OverwriteTarget::Everyone, 0, VIEW_CHANNEL)],
            },
        )
        .unwrap();
        let s = c.snapshot(30).unwrap();
        assert!(s.channels.is_empty() && s.categories.is_empty());
        assert_eq!(c.snapshot(10).unwrap().channels.len(), 1);
        assert!(
            c.clone()
                .mutate(
                    10,
                    CommunityCommand::SetOverwrites {
                        channel_id: Some(ch),
                        category_id: None,
                        overwrites: vec![]
                    }
                )
                .is_err()
        );
        c.mutate(
            10,
            CommunityCommand::EditChannel {
                id: ch,
                name: "public".into(),
                category_id: Some(cat),
                position: 0,
                synced: false,
            },
        )
        .unwrap();
        assert_eq!(c.snapshot(30).unwrap().channels.len(), 1);
        assert!(
            c.clone()
                .mutate(10, CommunityCommand::DeleteCategory { id: cat })
                .is_err()
        );
    }
    #[test]
    fn ban_leave_role_cleanup_and_validation() {
        let mut c = fixture();
        c.mutate(10, CommunityCommand::Ban { user_id: 20 }).unwrap();
        assert!(c.join(20).is_err());
        assert!(c.snapshot(20).is_err());
        c.mutate(10, CommunityCommand::Unban { user_id: 20 })
            .unwrap();
        c.join(20).unwrap();
        assert!(c.member(20).unwrap().role_ids.is_empty());
        c.mutate(20, CommunityCommand::Leave).unwrap();
        assert!(c.clone().mutate(10, CommunityCommand::Leave).is_err());
        c.members[1].role_ids.push(999);
        assert!(c.validate().is_err());
    }
    #[test]
    fn invalid_masks_overwrites_and_bounds() {
        let c = fixture();
        for os in [
            vec![ow(OverwriteTarget::Everyone, VIEW_CHANNEL, VIEW_CHANNEL)],
            vec![ow(OverwriteTarget::Role(0), 0, VIEW_CHANNEL)],
            vec![ow(OverwriteTarget::Member(999), VIEW_CHANNEL, 0)],
            vec![ow(OverwriteTarget::Everyone, ADMINISTRATOR, 0)],
            vec![
                ow(OverwriteTarget::Everyone, 0, 0),
                ow(OverwriteTarget::Everyone, 0, 0),
            ],
        ] {
            assert!(c.valid_overwrites(&os).is_err());
        }
        for title in ["", " ", "bad\nname"] {
            assert!(!valid_name(title));
        }
        assert!(!valid_name(&"x".repeat(101)));
        assert!(
            c.clone()
                .mutate(
                    10,
                    CommunityCommand::CreateRole {
                        name: "x".into(),
                        permissions: 1 << 40,
                        position: 1
                    }
                )
                .is_err()
        );
    }
    #[test]
    fn official_is_read_only_and_owner_safe() {
        let mut c = Community::new(1, 10, "Porch".into(), true).unwrap();
        c.join(20).unwrap();
        assert_eq!(
            c.base(20) & (SEND_MESSAGES | CREATE_INVITE | MANAGE_ROLES),
            0
        );
        assert!(c.clone().mutate(10, CommunityCommand::Delete).is_err());
        assert!(
            c.clone()
                .mutate(10, CommunityCommand::TransferOwnership { user_id: 20 })
                .is_err()
        );
        assert!(c.mutate(10, CommunityCommand::Ban { user_id: 10 }).is_err());
    }
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn implicit_permissions_never_invent_bits(bits in any::<u64>()) {
            let result=implicit(bits);
            prop_assert_eq!(result & !bits,0);
            prop_assert_eq!(result & !ALL,0);
            if result&VIEW_CHANNEL==0 {prop_assert_eq!(result,0);}
            if result&SEND_MESSAGES==0 {prop_assert_eq!(result&SEND_DEPENDENT,0);}
            if result&CONNECT==0 {prop_assert_eq!(result&CONNECT_DEPENDENT,0);}
        }
        #[test]
        fn aggregate_roles_are_order_independent(a in 0u64..ALL,b in 0u64..ALL) {
            let c=fixture();
            let os=vec![ow(OverwriteTarget::Role(1),a&!ADMINISTRATOR,0),ow(OverwriteTarget::Role(2),0,b&!ADMINISTRATOR)];
            let mut reversed=os.clone();reversed.reverse();
            prop_assert_eq!(c.effective(20,&os),c.effective(20,&reversed));
        }
        #[test]
        fn member_deny_always_wins_role_allow(bits in 0u64..ALL) {
            let c=fixture();let mask=bits&!ADMINISTRATOR;
            let os=vec![ow(OverwriteTarget::Role(1),mask,0),ow(OverwriteTarget::Member(20),0,mask)];
            prop_assert_eq!(c.effective(20,&os)&mask,0);
        }
        #[test]
        fn hidden_view_implies_no_channel_permission(bits in 0u64..ALL) {
            let c=fixture();
            let os=vec![ow(OverwriteTarget::Role(1),bits&!ADMINISTRATOR,0),ow(OverwriteTarget::Member(20),0,VIEW_CHANNEL)];
            prop_assert_eq!(c.effective(20,&os),0);
        }
        #[test]
        fn random_role_creation_keeps_dense_order(positions in proptest::collection::vec(any::<usize>(),0..70)) {
            let mut c=fixture();
            for p in positions {
                let position=1+p%c.roles.len();
                c.mutate(10,CommunityCommand::CreateRole{name:"role".into(),permissions:0,position}).unwrap();
                prop_assert!(c.validate().is_ok());
            }
        }
    }
}
