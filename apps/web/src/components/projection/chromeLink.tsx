'use client';

import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';

/**
 * `next/link` for the projection chrome.
 *
 * Passed to every `ChromeBar` as `renderLink`. Without it the chrome renders
 * plain anchors, which are correct for the design system in isolation — real
 * anchors give middle-click and open-in-new-tab — but are a FULL DOCUMENT LOAD
 * in this app. The flat chrome used `next/link`; after the port every nav and
 * account click reloaded the page. Measured with a `window` marker that did not
 * survive the click.
 *
 * This keeps both: a real `href` for the browser, a client transition for the
 * reader.
 */
export function chromeLink(props: {
  href: string;
  children: React.ReactNode;
  onClick?: () => void;
  'aria-current'?: 'page';
  role?: string;
  className?: string;
}): React.ReactNode {
  const { href, children, onClick, ...rest } = props;
  return (
    <Link href={href as Route} onClick={onClick} {...rest}>
      {children}
    </Link>
  );
}
