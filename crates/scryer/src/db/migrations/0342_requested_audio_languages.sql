-- Audio languages a request or a list route asks its new title to carry.
-- Each column holds a JSON array of language codes (`original` stays as the
-- token), or NULL when the title inherits its library's and facet's.
-- Approval and list adds copy the values into the title's own settings.
ALTER TABLE media_requests ADD COLUMN requested_required_audio_languages_json TEXT;
ALTER TABLE media_requests ADD COLUMN requested_preferred_audio_languages_json TEXT;

ALTER TABLE list_subscription_routes ADD COLUMN required_audio_languages_json TEXT;
ALTER TABLE list_subscription_routes ADD COLUMN preferred_audio_languages_json TEXT;
