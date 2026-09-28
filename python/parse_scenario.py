import json
import sys
import AoE2ScenarioParser
from AoE2ScenarioParser.scenarios.aoe2_de_scenario import AoE2DEScenario
from AoE2ScenarioParser import settings

def enum_name_or_value(value):
    if hasattr(value, "name"):
        return value.name

    return value

settings.PRINT_STATUS_UPDATES = False

path = sys.argv[1]
scenario = AoE2DEScenario.from_file(path)

height = scenario.map_manager.map_height
width = scenario.map_manager.map_width
terrain = []
elevation = []
for y in range(height):
    for x in range(width):
        tile = scenario.map_manager.get_tile(x=x, y=y)
        terrain.append(tile.terrain_id)
        elevation.append(tile.elevation)

players = []
for player in scenario.player_manager.players:
    players.append({
        "id": int(player.player_id),
        "name": player.tribe_name,
        "active": player.active,
        "human": player.human,
        "color": player.color,
        "civilization": enum_name_or_value(player.civilization),
        "starting_age": enum_name_or_value(player.starting_age),
        "population_cap": player.population_cap,
        "resources": {
            "food": player.food,
            "wood": player.wood,
            "gold": player.gold,
            "stone": player.stone,
        }
    })

units = []
for unit in scenario.unit_manager.get_all_units():
    units.append({
        "player": int(unit.player),
        "id": unit.reference_id,
        "type_id": unit.unit_const,
        "name": unit.name,
        "x": unit.x,
        "y": unit.y,
    })

messages = {
    "instructions": scenario.message_manager.instructions,
    "hints": scenario.message_manager.hints,
    "victory": scenario.message_manager.victory,
    "loss": scenario.message_manager.loss,
    "history": scenario.message_manager.history,
    "scouts": scenario.message_manager.scouts,
}

result = {
    "parser_version": AoE2ScenarioParser.__version__,
    "scenario_version": scenario.scenario_version,
    "game_version": scenario.game_version,
    "width": width,
    "height": height,
    "terrain": terrain,
    "elevation": elevation,
    "players": players,
    "units": units,
    "messages": messages,
}

print(json.dumps(result))