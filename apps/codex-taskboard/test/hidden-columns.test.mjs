import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const appSource = await readFile(new URL("../web/src/App.tsx", import.meta.url), "utf8");
const settingsSource = await readFile(new URL("../web/src/components/BoardSettingsMenu.tsx", import.meta.url), "utf8");
const boardColumnSource = await readFile(new URL("../web/src/components/BoardColumn.tsx", import.meta.url), "utf8");
const hiddenColumnsSource = await readFile(new URL("../web/src/components/HiddenColumns.tsx", import.meta.url), "utf8");
const visibilityMenuSource = await readFile(new URL("../web/src/components/ColumnVisibilityMenu.tsx", import.meta.url), "utf8");
const iconSource = await readFile(new URL("../web/src/components/LinearIcon.tsx", import.meta.url), "utf8");
const styles = await readFile(new URL("../web/src/styles.css", import.meta.url), "utf8");

test("empty workflow statuses are partitioned from the filtered board result", () => {
  assert.match(appSource, /const visibleStatuses = useMemo\([\s\S]*?tasksByStatus\[status\]\.length === 0[\s\S]*?\? showEmptyColumns[\s\S]*?: \(columnVisibility\?\.\[status\] \?\? true\)/);
  assert.match(appSource, /const hiddenStatuses = useMemo\([\s\S]*?tasksByStatus\[status\]\.length === 0[\s\S]*?\? !showEmptyColumns[\s\S]*?: !\(columnVisibility\?\.\[status\] \?\? true\)/);
  assert.match(appSource, /visibleStatuses\.map\(\(status\) =>/);
  assert.match(appSource, /statusIndex=\{TASK_STATUSES\.indexOf\(status\)\}/);
  assert.match(appSource, /hiddenStatuses\.length > 0[\s\S]*?<HiddenColumns/);
  assert.doesNotMatch(appSource, /\) : filteredTasks\.length === 0 && tasks\.length > 0 \? \(/);
  assert.match(appSource, /<div className="board">[\s\S]*?filteredTasks\.length === 0[\s\S]*?<HiddenColumns/);
});

test("show empty columns is off by default, persisted locally, and updates immediately", () => {
  assert.match(appSource, /const SHOW_EMPTY_COLUMNS_KEY = "taskboard\.showEmptyColumns\.v1"/);
  assert.match(appSource, /getItem\(SHOW_EMPTY_COLUMNS_KEY\) === "true"/);
  assert.match(appSource, /useState\(readShowEmptyColumns\)/);
  assert.match(appSource, /localStorage\.setItem\(SHOW_EMPTY_COLUMNS_KEY, String\(show\)\)/);
  assert.match(appSource, /setShowEmptyColumns\(show\)/);
  assert.match(settingsSource, /role="switch"/);
  assert.match(settingsSource, /aria-checked=\{showEmptyColumns\}/);
  assert.match(settingsSource, />显示空列</);
});

test("column menus persist manual hiding and showing per project", () => {
  assert.match(appSource, /const COLUMN_VISIBILITY_KEY = "taskboard\.columnVisibility\.v1"/);
  assert.match(appSource, /useState\(readColumnVisibilityByProject\)/);
  assert.match(appSource, /window\.localStorage\.setItem\(COLUMN_VISIBILITY_KEY, JSON\.stringify\(next\)\)/);
  assert.match(appSource, /if \(!selectedProjectId \|\| tasksByStatus\[status\]\.length === 0\) return/);
  assert.match(appSource, /onHide=\{\(hiddenStatus\) => updateColumnVisibility\(hiddenStatus, false\)\}/);
  assert.match(appSource, /onShow=\{\(shownStatus\) => updateColumnVisibility\(shownStatus, true\)\}/);
  assert.match(boardColumnSource, /tasks\.length > 0[\s\S]*?action="hide"/);
  assert.match(boardColumnSource, /onAction=\{\(\) => onHide\(status\)\}/);
  assert.match(hiddenColumnsSource, /const canShow = counts\[status\] > 0/);
  assert.match(hiddenColumnsSource, /canShow[\s\S]*?action="show"/);
  assert.match(hiddenColumnsSource, /onClick=\{\(\) => onShow\(status\)\}/);
  assert.match(hiddenColumnsSource, /className="hidden-column-show is-empty"/);
  assert.match(hiddenColumnsSource, /\{counts\[status\]\}/);
  assert.match(visibilityMenuSource, /action === "hide" \? "隐藏列" : "显示列"/);
  assert.match(visibilityMenuSource, /createPortal/);
  assert.match(styles, /\.column-visibility-menu/);
  assert.match(styles, /\.hidden-column-show/);
});

test("隐藏列 is collapsible and accepts drops into hidden statuses", () => {
  assert.match(hiddenColumnsSource, /aria-controls="hidden-columns-list"/);
  assert.match(hiddenColumnsSource, /aria-expanded=\{open\}/);
  assert.match(hiddenColumnsSource, /aria-label="隐藏列"/);
  assert.match(hiddenColumnsSource, />隐藏列</);
  assert.doesNotMatch(hiddenColumnsSource, /statuses\.length/);
  assert.match(hiddenColumnsSource, /application\/x-taskboard-task/);
  assert.match(hiddenColumnsSource, /onDrop\(status, taskId\)/);
  assert.match(appSource, /onDrop=\{\(destination, taskId\) => finishTaskDrop\(destination, taskId\)\}/);
  assert.match(styles, /\.hidden-column-item\.is-drop-target/);
});

test("display options uses the centralized Linear sliders asset", () => {
  assert.match(settingsSource, /<LinearIcon name="displayOptions" \/>/);
  assert.doesNotMatch(settingsSource, /<svg/);
  assert.match(iconSource, /displayOptions: \{/);
  assert.match(iconSource, /M7 2\.5C8\.11933 2\.5/);
  assert.match(styles, /\.board-settings-menu/);
  assert.match(styles, /\.board-setting-switch\.is-on/);
});

test("settings focus enters the portal and returns to the trigger on Escape", () => {
  assert.match(settingsSource, /requestAnimationFrame\(\(\) => menuRef\.current\?\.querySelector<HTMLButtonElement>\("\[role='switch'\]"\)\?\.focus\(\)\)/);
  assert.match(settingsSource, /event\.key === "Escape"[\s\S]*?triggerRef\.current\?\.focus\(\)/);
  assert.match(settingsSource, /event\.key === "Tab"[\s\S]*?event\.preventDefault\(\)/);
});
