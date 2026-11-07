const DAILY_RELOAD_DELAY_MS = 5000;
const CLOCK_MS = 30_000;
const BACKGROUND_CHECK_MS = 3_600_000;
const SEARCH_WAIT_MS = 250;
const shopState = { view: null, loading: false, checked: false, clock: null, wished: new Set(), searchTimer: null };

function formatRemaining(seconds) {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours >= 48) return `残り ${Math.floor(hours / 24)}日`;
  return hours > 0 ? `残り ${hours}時間${minutes}分` : `残り ${minutes}分`;
}

function countdown(seconds, className = "shop-clock") {
  const node = element("span", className, formatRemaining(seconds));
  node.dataset.ends = String(Date.now() + seconds * 1000);
  return node;
}

function formatVp(value) {
  return value === null || value === undefined ? "価格不明" : `${value.toLocaleString("ja-JP")} VP`;
}

function seenText(seen) {
  if (!seen.times) return "この記録では初めて";
  const date = new Intl.DateTimeFormat("ja-JP", { month: "numeric", day: "numeric" }).format(new Date(seen.last_ms));
  return `過去 ${seen.times}回、前回 ${date}`;
}

function wishButton(item) {
  const button = element("button", "wish");
  button.type = "button";
  const sync = () => {
    const wished = shopState.wished.has(item.item_id);
    button.textContent = wished ? "欲しい物から外す" : "欲しい物に追加";
    button.setAttribute("aria-pressed", String(wished));
  };
  sync();
  button.addEventListener("click", async () => {
    const wished = !shopState.wished.has(item.item_id);
    const entry = { item_id: item.item_id, name: item.name, image: item.image ?? null };
    await window.__TAURI__.core.invoke("set_wish", { item: entry, wished });
    if (wished) shopState.wished.add(item.item_id);
    else shopState.wished.delete(item.item_id);
    sync();
    renderWishlist();
  });
  return button;
}

function affordText(item, wallet) {
  if (item.price === null || item.price === undefined) return null;
  const short = item.price - wallet.vp;
  return short > 0 ? `あと ${short.toLocaleString("ja-JP")} VP 足りません` : "手持ちで買えます";
}

function skinCard(item, wallet) {
  const card = element("article", item.wished ? "skin-card is-wished" : "skin-card");
  if (item.tier) card.style.setProperty("--tier", item.tier.color);
  const image = picture(item.image, "skin-image");
  if (image) card.append(image);
  card.append(element("h3", "", item.name));
  const price = element("p", "skin-price", formatVp(item.price));
  if (item.base_price) price.append(" ", element("s", "", formatVp(item.base_price)));
  if (item.discount_percent) price.append(" ", element("b", "", `-${item.discount_percent}%`));
  card.append(price);
  const facts = [item.tier?.name, affordText(item, wallet), seenText(item.seen)].filter(Boolean);
  card.append(element("small", "", facts.join("、")), wishButton(item));
  return card;
}

function bundleCard(bundle) {
  const card = element("article", "bundle-card halftone");
  const image = picture(bundle.image, "halftone-image");
  if (image) card.prepend(image);
  card.append(displayText("h3", bundle.name, "bundle-name"));
  const line = element("p", "bundle-line", `${formatVp(bundle.price)}、${bundle.items}点`);
  line.append("、", countdown(bundle.remaining_s));
  card.append(line);
  return card;
}

function shopSection(title, clock, children) {
  const section = element("section", "shop-section");
  const head = element("div", "shop-head");
  head.append(element("h2", "", title));
  if (clock) head.append(clock);
  section.append(head, ...children);
  return section;
}

function renderWishlist() {
  const list = document.getElementById("wish-list");
  if (!list) return;
  window.__TAURI__.core.invoke("wishlist").then((items) => {
    shopState.wished = new Set(items.map((item) => item.item_id));
    list.replaceChildren(...items.map((item) => {
      const row = element("li");
      const image = picture(item.image, "wish-image");
      if (image) row.append(image);
      row.append(element("span", "", item.name), wishButton(item));
      return row;
    }));
    if (items.length === 0) list.replaceChildren(element("li", "lobby-note", "スキンを追加すると、ストアに出たときに通知します。"));
  });
}

function wishSearch() {
  const input = element("input", "wish-search");
  input.type = "search";
  input.placeholder = "スキン名で探す";
  input.setAttribute("aria-label", "スキン名で探す");
  const results = element("ul", "wish-results");
  input.addEventListener("input", () => {
    clearTimeout(shopState.searchTimer);
    shopState.searchTimer = setTimeout(async () => {
      const found = await window.__TAURI__.core.invoke("search_skins", { query: input.value });
      results.replaceChildren(...found.map((skin) => {
        const row = element("li");
        const image = picture(skin.image, "wish-image");
        if (image) row.append(image);
        row.append(element("span", "", skin.name), wishButton(skin));
        return row;
      }));
    }, SEARCH_WAIT_MS);
  });
  const list = element("ul", "wish-results");
  list.id = "wish-list";
  return [input, results, list];
}

function renderShop(view, shop) {
  const hero = element("section", "shop-hero");
  const wallet = element("div", "hero-figures");
  wallet.append(
    figure(shop.wallet.vp.toLocaleString("ja-JP"), "VALORANT POINTS"),
    figure(shop.wallet.radianite.toLocaleString("ja-JP"), "レディアナイト"),
    figure(shop.wallet.kingdom.toLocaleString("ja-JP"), "キングダムクレジット"),
  );
  hero.append(displayText("h2", "STORE", "shop-title"), wallet);

  const total = shop.daily.reduce((sum, item) => sum + (item.price ?? 0), 0);
  const dailyGrid = element("div", "skin-grid");
  dailyGrid.append(...shop.daily.map((item) => skinCard(item, shop.wallet)));
  const sections = [shopSection(`今日のストア 全部で ${formatVp(total)}`, countdown(shop.daily_remaining_s), [dailyGrid])];
  if (shop.night_market.length > 0) {
    const grid = element("div", "skin-grid");
    grid.append(...shop.night_market.map((item) => skinCard(item, shop.wallet)));
    sections.push(shopSection("ナイトマーケット", countdown(shop.night_remaining_s ?? 0), [grid]));
  }
  if (shop.bundles.length > 0) {
    sections.push(shopSection("バンドル", null, shop.bundles.map(bundleCard)));
  }
  const body = element("div", "shop-body");
  const main = element("div");
  main.append(...sections);
  body.append(main, shopSection("欲しい物", null, wishSearch()));
  view.replaceChildren(hero, body);
  renderWishlist();
}

async function fetchShop() {
  const shop = await window.__TAURI__.core.invoke("load_shop");
  shopState.view = shop;
  shopState.wished = new Set([...shop.daily, ...shop.night_market].filter((item) => item.wished).map((item) => item.item_id));
  return shop;
}

async function showShop(view) {
  if (shopState.loading) return;
  shopState.loading = true;
  if (!shopState.view) view.replaceChildren(noticeBox("ストアを読み込んでいます", "スキンの情報を初めて取得するときは、少し時間がかかります。"));
  try {
    renderShop(view, await fetchShop());
  } catch (error) {
    view.replaceChildren(noticeBox("ストアを開けませんでした", `VALORANT が起動しているか確認してください。${error}`));
  } finally {
    shopState.loading = false;
  }
}

/** 欲しいスキンの通知のため、画面を開いていなくても確認する。 */
function checkShopInBackground() {
  if (shopState.checked) return;
  shopState.checked = true;
  const check = () => fetchShop().catch(() => { shopState.checked = false; });
  check();
  setInterval(check, BACKGROUND_CHECK_MS);
}

setInterval(() => {
  const clocks = [...document.querySelectorAll("#shop-view [data-ends]")];
  const now = Date.now();
  for (const clock of clocks) clock.textContent = formatRemaining(Math.max(0, (Number(clock.dataset.ends) - now) / 1000));
  const expired = clocks.some((clock) => Number(clock.dataset.ends) + DAILY_RELOAD_DELAY_MS < now);
  if (expired && !document.getElementById("shop-view").hidden) showShop(document.getElementById("shop-view"));
}, CLOCK_MS);
