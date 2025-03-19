use std::collections::HashMap;

#[derive(Debug, Clone)]
struct Answer {
    id: usize,
    text: Vec<String>,
    response_counter: usize,
    parent_question_id: usize,
}

#[derive(Debug, Clone)]
struct Question {
    id: usize,
    text: Vec<String>,
    answers: Vec<usize>, // Store answer IDs
    current_answer_id: usize,
    parent_question_id: Option<usize>,
    children_question_ids: Vec<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct ConversationTree {
    questions: HashMap<usize, Question>,
    answers: HashMap<usize, Answer>,
    current_question_ids: Vec<usize>,
    at_question: usize,
    next_question_id: usize,
    next_answer_id: usize,
}

impl ConversationTree {
    pub(crate) fn new() -> Self {
        ConversationTree {
            questions: HashMap::new(),
            answers: HashMap::new(),
            current_question_ids: vec![0],
            at_question: 0,
            next_question_id: 1,
            next_answer_id: 1,
        }
    }

    pub(crate) fn add_question(&mut self, question_text: Vec<String>) -> usize {
        let question_id = self.next_question_id;
        let answer_id = self.next_answer_id;

        let question = Question {
            id: question_id,
            text: question_text,
            answers: vec![answer_id],
            current_answer_id: answer_id,
            parent_question_id: None,
            children_question_ids: Vec::new(),
        };

        let answer = Answer {
            id: answer_id,
            text: Vec::new(),
            response_counter: 0,
            parent_question_id: question_id,
        };

        self.questions.insert(question_id, question);
        self.answers.insert(answer_id, answer);

        self.current_question_ids.push(question_id);
        self.at_question += 1;
        self.next_question_id += 1;
        self.next_answer_id += 1;
        question_id
    }

    pub(crate) fn answer(&mut self, question_id: usize, answer_text: Vec<String>) {
        if let Some(question) = self.questions.get_mut(&question_id) {
            let answer_id = question.current_answer_id;
            let answer = self.answers.get_mut(&answer_id);
            if let Some(answer) = answer {
                answer.text = answer_text;
                answer.response_counter += 1;
            }
        }
    }

    pub(crate) fn add_q_and_a(&mut self, question_text: Vec<String>, answer_text: Vec<String>) {
        let question_id = self.next_question_id;
        let answer_id = self.next_answer_id;

        let question = Question {
            id: question_id,
            text: question_text,
            answers: vec![answer_id],
            current_answer_id: answer_id,
            parent_question_id: None,
            children_question_ids: Vec::new(),
        };

        let answer = Answer {
            id: answer_id,
            text: answer_text,
            response_counter: 1,
            parent_question_id: question_id,
        };

        self.questions.insert(question_id, question);
        self.answers.insert(answer_id, answer);

        self.current_question_ids.push(question_id);
        self.at_question += 1;
        self.next_question_id += 1;
        self.next_answer_id += 1;
    }

    pub(crate) fn edit_question(
        &mut self,
        question_id: usize,
        new_question_text: Vec<String>,
        new_answer_text: Vec<String>,
    ) {
        if let Some(question) = self.questions.get(&question_id) {
            let new_question_id = self.next_question_id;
            let new_answer_id = self.next_answer_id;

            let new_question = Question {
                id: new_question_id,
                text: new_question_text,
                answers: vec![new_answer_id],
                current_answer_id: new_answer_id,
                parent_question_id: Some(question_id),
                children_question_ids: Vec::new(),
            };

            let new_answer = Answer {
                id: new_answer_id,
                text: new_answer_text,
                response_counter: 1,
                parent_question_id: new_question_id,
            };

            self.questions.insert(new_question_id, new_question);
            self.answers.insert(new_answer_id, new_answer);

            if let Some(parent_question) = self.questions.get_mut(&question_id) {
                parent_question.children_question_ids.push(new_question_id);
            }
            self.current_question_ids[self.at_question] = new_question_id;
            self.next_question_id += 1;
            self.next_answer_id += 1;
        }
    }

    pub(crate) fn add_answer(&mut self, question_id: usize, new_answer_text: Vec<String>) {
        if let Some(question) = self.questions.get_mut(&question_id) {
            let new_answer_id = self.next_answer_id;

            let new_answer = Answer {
                id: new_answer_id,
                text: new_answer_text,
                response_counter: question.answers.len() + 1,
                parent_question_id: question_id,
            };

            question.answers.push(new_answer_id);
            question.current_answer_id = new_answer_id;
            self.answers.insert(new_answer_id, new_answer);

            self.next_answer_id += 1;
        }
    }

    pub(crate) fn get_question_nr_of_total(&self, question_id: usize) -> String {
        if let Some(question) = self.questions.get(&question_id) {
            let parent = question.parent_question_id;
            let mut total = 0;
            let mut current = 0;
            let mut up = "";
            let mut down = "";
            for (id, q) in &self.questions {
                if q.parent_question_id == parent {
                    total += 1;
                    if *id == question_id {
                        current = total;
                    } else if current == 0 {
                        up = "▲";
                    } else {
                        down = "▼";
                    }
                }
            }
            format!("[{}{}/{}{}]", up, current, total, down)
        } else {
            String::from("[0/0]")
        }
    }

    pub(crate) fn get_answer_nr_of_total(&self, question_id: usize, answer_id: usize) -> String {
        if let Some(answer) = self
            .questions
            .get(&question_id)
            .and_then(|question| question.answers.get(answer_id))
            .and_then(|answer_id| self.answers.get(answer_id))
        {
            let parent = answer.parent_question_id;
            let mut total = 0;
            let mut current = 0;
            let mut up = "";
            let mut down = "";
            for (id, a) in &self.answers {
                if a.parent_question_id == parent {
                    total += 1;
                    if *id == answer_id {
                        current = total;
                    } else if current == 0 {
                        up = "▲";
                    } else {
                        down = "▼";
                    }
                }
            }
            format!("[{}{}/{}{}]", up, current, total, down)
        } else {
            String::from("[0/0]")
        }
    }

    pub(crate) fn find_parent(&self, question_id: usize) -> Option<usize> {
        if let Some(question) = self.questions.get(&question_id) {
            question.parent_question_id
        } else {
            None
        }
    }

    pub(crate) fn get_qa(
        &self,
        question_id: usize,
        answer_id: usize,
    ) -> Option<(Vec<String>, Vec<String>)> {
        if let Some(question) = self.questions.get(&question_id) {
            let question_text = question.text.clone();
            let response_text = self
                .answers
                .get(&answer_id)
                .map_or(Vec::new(), |answer| answer.text.clone());
            Some((question_text, response_text))
        } else {
            None
        }
    }

    pub(crate) fn get_current_question_id(&self) -> usize {
        self.current_question_ids[self.at_question]
    }

    pub(crate) fn get_current_answer_id(&self, current_question_id: usize) -> usize {
        self.questions
            .get(&current_question_id)
            .map_or(0, |question| question.current_answer_id)
    }
}
