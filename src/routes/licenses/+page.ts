// Tauri のウインドウ (WebviewUrl::App("licenses")) が開く build/licenses.html を出すため、
// 中身の無い殻を prerender する。ssr はルートの layout で切ってある
export const prerender = true;
