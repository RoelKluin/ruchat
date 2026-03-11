pub(crate) struct Context {
    goal: String,
    pub(crate) history: String,
    pub(crate) output: String,
    pub(crate) context: String,
    pub(crate) rejections: String,
    pub(crate) documents: String,
}

impl Context {
    pub(crate) fn new(goal: String) -> Self {
        Self {
            goal: format!("Goal: {goal}\n\n"),
            history: String::new(),
            output: String::new(),
            context: String::new(),
            rejections: String::new(),
            documents: String::new(),
        }
    }
    pub(crate) fn get_goal(&self) -> &str {
        &self.goal
    }
    pub(crate) fn is_approved(&self) -> bool {
        self.rejections.is_empty()
    }
}
