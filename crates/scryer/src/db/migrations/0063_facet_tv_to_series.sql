-- Rename facet "tv" to "series" for consistency with MediaFacet::Series.
UPDATE titles SET facet = 'series' WHERE facet = 'tv';
