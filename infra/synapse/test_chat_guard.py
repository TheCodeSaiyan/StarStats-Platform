# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for starstats_chat_guard, run inside the Synapse image:

    docker build -t ss-synapse infra/synapse
    docker run --rm --entrypoint python ss-synapse -m unittest -v test_chat_guard

Uses the real Synapse NOT_SPAM and Codes, so a change in the module API
shape fails here rather than in production.
"""

import asyncio
import unittest

from synapse.api.errors import Codes
from synapse.module_api import NOT_SPAM

from starstats_chat_guard import DAY_MS, ChatGuard

SERVICE = "@starstats:starstats.app"
PLAYER = "@wingman:starstats.app"
ROOM = "!room:starstats.app"


class FakeApi:
    def __init__(self):
        self.callbacks = {}

    def register_spam_checker_callbacks(self, **kw):
        self.callbacks = kw


def guard():
    api = FakeApi()
    g = ChatGuard(
        ChatGuard.parse_config({"service_user": SERVICE, "max_lifetime_days": 90}), api
    )
    return g, api


def run(coro):
    return asyncio.run(coro)


class ChatGuardTests(unittest.TestCase):
    def test_registers_every_callback_it_relies_on(self):
        _, api = guard()
        self.assertEqual(
            set(api.callbacks),
            {
                "user_may_create_room",
                "user_may_invite",
                "user_may_join_room",
                "user_may_send_3pid_invite",
                "user_may_create_room_alias",
                "user_may_publish_room",
                "user_may_send_state_event",
            },
        )

    def test_only_the_service_account_creates_rooms_and_invites(self):
        g, _ = guard()
        self.assertEqual(run(g.user_may_create_room(SERVICE, {})), NOT_SPAM)
        self.assertEqual(run(g.user_may_create_room(PLAYER, {})), Codes.FORBIDDEN)
        self.assertEqual(run(g.user_may_create_room(PLAYER)), Codes.FORBIDDEN, "one-arg form")
        self.assertEqual(run(g.user_may_invite(SERVICE, PLAYER, ROOM)), NOT_SPAM)
        self.assertEqual(
            run(g.user_may_invite(PLAYER, "@stranger:starstats.app", ROOM)),
            Codes.FORBIDDEN,
            "a player cannot start a DM with anyone",
        )
        self.assertEqual(run(g.user_may_create_room_alias(PLAYER, "#x:starstats.app")), Codes.FORBIDDEN)
        self.assertEqual(run(g.user_may_publish_room(PLAYER, ROOM)), Codes.FORBIDDEN)

    def test_a_player_joins_only_what_they_were_invited_to(self):
        g, _ = guard()
        self.assertEqual(run(g.user_may_join_room(PLAYER, ROOM, True)), NOT_SPAM)
        self.assertEqual(run(g.user_may_join_room(PLAYER, ROOM, False)), Codes.FORBIDDEN)
        self.assertEqual(run(g.user_may_join_room(SERVICE, ROOM, False)), NOT_SPAM)

    def test_no_email_invites_for_anyone(self):
        g, _ = guard()
        for who in (SERVICE, PLAYER):
            self.assertEqual(
                run(g.user_may_send_3pid_invite(who, "email", "a@b.c", ROOM)), Codes.FORBIDDEN
            )

    def test_players_set_only_membership_and_a_bounded_retention(self):
        g, _ = guard()
        state = lambda t, c: run(g.user_may_send_state_event(PLAYER, ROOM, t, "", c))
        self.assertEqual(state("m.room.member", {"membership": "leave"}), NOT_SPAM)
        for t in ("m.room.name", "m.room.topic", "m.room.avatar", "m.room.join_rules",
                  "m.room.power_levels", "m.room.encryption", "m.room.history_visibility"):
            self.assertEqual(state(t, {}), Codes.FORBIDDEN, t)
        self.assertEqual(state("m.room.retention", {"max_lifetime": DAY_MS}), NOT_SPAM)
        self.assertEqual(state("m.room.retention", {"max_lifetime": 90 * DAY_MS}), NOT_SPAM)
        self.assertEqual(state("m.room.retention", {"max_lifetime": 91 * DAY_MS}), Codes.FORBIDDEN)
        self.assertEqual(state("m.room.retention", {"max_lifetime": DAY_MS - 1}), Codes.FORBIDDEN)
        self.assertEqual(state("m.room.retention", {}), Codes.FORBIDDEN)
        self.assertEqual(state("m.room.retention", {"max_lifetime": True}), Codes.FORBIDDEN)
        self.assertEqual(
            state("m.room.retention", {"max_lifetime": DAY_MS, "min_lifetime": DAY_MS}),
            Codes.FORBIDDEN,
            "no min_lifetime: it would hold messages past the maximum",
        )
        self.assertEqual(
            run(g.user_may_send_state_event(SERVICE, ROOM, "m.room.name", "", {})), NOT_SPAM
        )

    def test_bad_config_is_refused_at_startup(self):
        with self.assertRaises(ValueError):
            ChatGuard.parse_config({"service_user": "starstats", "max_lifetime_days": 90})
        with self.assertRaises(ValueError):
            ChatGuard.parse_config({"service_user": SERVICE, "max_lifetime_days": 0})


if __name__ == "__main__":
    unittest.main()
