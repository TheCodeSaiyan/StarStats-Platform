import { heatmapWidget } from './heatmap';
import { orgsWidget } from './orgs';
import { sessionsWidget } from './sessions';
import { hangarWidget } from './hangar';
import { loadoutWidget } from './loadout';
import { entitiesWidget } from './entities';
import { combatMissionWidget } from './combat_mission';
import { economyWidget } from './economy';
import { travelWidget } from './travel';
import { journeyWidget } from './journey';
import { recordsWidget } from './records';
import { recentActivityWidget } from './recent_activity';
import { livesWidget } from './lives';
import { fleetWidget } from './fleet';
import { dockingWidget } from './docking';
import { objectivesWidget } from './objectives';
import { contractsWidget } from './contracts';
import { spendWidget } from './spend';
import { routesWidget } from './routes';
import { locationsWidget } from './locations';
import { corridorsWidget } from './corridors';
import { factsWidget } from './facts';
import type { WidgetDef, WidgetId } from './types';

export const WIDGETS: readonly WidgetDef[] = [
  sessionsWidget,
  heatmapWidget,
  orgsWidget,
  recentActivityWidget,
  combatMissionWidget,
  economyWidget,
  travelWidget,
  journeyWidget,
  recordsWidget,
  hangarWidget,
  loadoutWidget,
  entitiesWidget,
  livesWidget,
  fleetWidget,
  dockingWidget,
  objectivesWidget,
  contractsWidget,
  spendWidget,
  routesWidget,
  locationsWidget,
  corridorsWidget,
  factsWidget,
];

export const WIDGETS_BY_ID: ReadonlyMap<WidgetId, WidgetDef> = new Map(
  WIDGETS.map((w) => [w.id, w] as const),
);

export const REGISTERED_IDS: readonly WidgetId[] = WIDGETS.map((w) => w.id);
