import type { Component } from 'svelte';
import type { SectionProps } from './form';
import BrowserAdvanced from './sections/BrowserAdvanced.svelte';
import Captcha from './sections/Captcha.svelte';
import Content from './sections/Content.svelte';
import Display from './sections/Display.svelte';
import Extensions from './sections/Extensions.svelte';
import Idle from './sections/Idle.svelte';
import Locale from './sections/Locale.svelte';
import LocaleAdvanced from './sections/LocaleAdvanced.svelte';
import Network from './sections/Network.svelte';
import NetworkAdvanced from './sections/NetworkAdvanced.svelte';
import Storage from './sections/Storage.svelte';
import Version from './sections/Version.svelte';

type Section = { id: string; title: string; templateTitle?: string; description?: string; component: Component<SectionProps> };

export const BASIC_SECTIONS: Section[] = [
  { id: 'version', title: 'Browser version', component: Version },
  { id: 'display', title: 'Display', component: Display },
  { id: 'network', title: 'Proxy', component: Network },
  { id: 'locale', title: 'Language & time zone', component: Locale },
  { id: 'extensions', title: 'Extensions', component: Extensions },
  { id: 'captcha', title: 'Captcha', component: Captcha },
  { id: 'content', title: 'Content', component: Content },
  { id: 'storage', title: 'Storage & retention', component: Storage },
  { id: 'idle', title: 'Idle & pinning', templateTitle: 'Idle', component: Idle },
];

export const ADVANCED_SECTIONS: Section[] = [
  { id: 'launch-options', title: 'Launch options', component: BrowserAdvanced },
  { id: 'network-privacy', title: 'Network privacy', component: NetworkAdvanced },
  { id: 'location', title: 'Location, window & fingerprint', component: LocaleAdvanced },
];

export const ADVANCED_NOTE = 'Changing these can make the browser easier to detect.';
