# SPDX-License-Identifier: AGPL-3.0-or-later
#
# This module runs inside Synapse, which is AGPL-3.0, so it is published
# under the same licence. Its source is this file in the public StarStats
# repository.
"""StarStats chat guard: the rules Synapse cannot know, enforced in Synapse.

StarStats decides who may talk to whom (friends-only DMs, the accepted crew
of a Looking for Group post, blocks, the ``chat`` account restriction).
Synapse knows none of that, and clients cannot be trusted to respect it, so
this module makes the StarStats API the only thing that can shape a room:

- only the StarStats service account (the API's application service) may
  create rooms, invite, create aliases or publish to the directory;
- a player may join only a room they were invited to, and only the service
  account invites, so every membership is one the API decided;
- third-party (email) invites are refused;
- a player may change no room state except their own membership and the
  room's retention period (the "temporary chat" setting), which must stay
  between one day and the server maximum. Room names, topics and avatars are
  refused because they are not end-to-end encrypted.

Revoking access (unfriend, block, removal from a crew, a restriction) is the
API kicking the player, so no rule here needs to call out to the API.

Synapse skips these checks only for server admins. The service account is
deliberately not an admin: it is recognised here by its user ID instead.
"""

from typing import Any, Literal

from synapse.api.errors import Codes
from synapse.module_api import NOT_SPAM, ModuleApi

DAY_MS = 24 * 60 * 60 * 1000

Verdict = Literal["NOT_SPAM"] | Codes

# State a player may set. Everything else in room state is server-visible,
# so it stays with the service account.
PLAYER_STATE = {"m.room.member", "m.room.retention"}


class ChatGuard:
    def __init__(self, config: dict[str, Any], api: ModuleApi):
        self._service = config["service_user"]
        self._max_lifetime_ms = int(config["max_lifetime_days"]) * DAY_MS
        api.register_spam_checker_callbacks(
            user_may_create_room=self.user_may_create_room,
            user_may_invite=self.user_may_invite,
            user_may_join_room=self.user_may_join_room,
            user_may_send_3pid_invite=self.user_may_send_3pid_invite,
            user_may_create_room_alias=self.user_may_create_room_alias,
            user_may_publish_room=self.user_may_publish_room,
            user_may_send_state_event=self.user_may_send_state_event,
        )

    @staticmethod
    def parse_config(config: dict[str, Any]) -> dict[str, Any]:
        if not str(config.get("service_user", "")).startswith("@"):
            raise ValueError("starstats_chat_guard: service_user must be a Matrix user ID")
        if int(config.get("max_lifetime_days", 0)) < 1:
            raise ValueError("starstats_chat_guard: max_lifetime_days must be at least 1")
        return config

    def _service_only(self, user_id: str) -> Verdict:
        return NOT_SPAM if user_id == self._service else Codes.FORBIDDEN

    async def user_may_create_room(self, user_id: str, *_: Any) -> Verdict:
        return self._service_only(user_id)

    async def user_may_invite(self, inviter: str, invitee: str, room_id: str) -> Verdict:
        return self._service_only(inviter)

    async def user_may_join_room(self, user_id: str, room_id: str, is_invited: bool) -> Verdict:
        if user_id == self._service or is_invited:
            return NOT_SPAM
        return Codes.FORBIDDEN

    async def user_may_send_3pid_invite(
        self, inviter: str, medium: str, address: str, room_id: str
    ) -> Verdict:
        return Codes.FORBIDDEN

    async def user_may_create_room_alias(self, user_id: str, room_alias: Any) -> Verdict:
        return self._service_only(user_id)

    async def user_may_publish_room(self, user_id: str, room_id: str) -> Verdict:
        return self._service_only(user_id)

    async def user_may_send_state_event(
        self,
        user_id: str,
        room_id: str,
        event_type: str,
        state_key: str,
        content: dict[str, Any],
    ) -> Verdict:
        if user_id == self._service:
            return NOT_SPAM
        if event_type not in PLAYER_STATE:
            return Codes.FORBIDDEN
        if event_type == "m.room.retention" and not self._retention_ok(content):
            return Codes.FORBIDDEN
        return NOT_SPAM

    def _retention_ok(self, content: dict[str, Any]) -> bool:
        """A retention period a player may choose: a max lifetime between a
        day and the server maximum, and no min lifetime (which would let
        a player make messages outlive the maximum)."""
        if "min_lifetime" in content:
            return False
        max_lifetime = content.get("max_lifetime")
        return (
            isinstance(max_lifetime, int)
            and not isinstance(max_lifetime, bool)
            and DAY_MS <= max_lifetime <= self._max_lifetime_ms
        )
