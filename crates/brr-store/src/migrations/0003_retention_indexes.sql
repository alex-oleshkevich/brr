CREATE INDEX event_instance ON event(json_extract(data, '$.instance_id'));
CREATE INDEX instance_live ON instance(id) WHERE state NOT IN ('stopped', 'failed', 'detached', 'lost');
CREATE INDEX instance_ended ON instance(ended_at) WHERE ended_at IS NOT NULL;
CREATE INDEX lease_released ON lease(released_at) WHERE released_at IS NOT NULL;
CREATE INDEX client_ended ON client(ended_at) WHERE ended_at IS NOT NULL;
